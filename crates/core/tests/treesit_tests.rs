use std::cell::RefCell;
use std::rc::Rc;

use core::editor::Editor;
use elisp::printer::prin1_to_string;
use elisp::Interp;

const RUST_SRC: &str =
    "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nfn main() {\n    let x = add(1, 2);\n}\n";

fn setup(src: &str) -> (Interp, Rc<RefCell<Editor>>) {
    let mut interp = elisp::new_interp();
    let ed = core::init_editor(&mut interp);
    run(&mut interp, &format!("(insert {:?})", src));
    (interp, ed)
}

fn run(interp: &mut Interp, src: &str) -> String {
    match interp.eval_source(src) {
        Ok(v) => prin1_to_string(interp, &v),
        Err(flow) => format!("ERROR: {}", interp.describe_flow(&flow)),
    }
}

#[test]
fn language_availability() {
    let (mut i, _ed) = setup(RUST_SRC);
    assert_eq!(run(&mut i, "(treesit-language-available-p 'rust)"), "t");
    // M25: python is now bundled too (see lang_modes_tests.rs for the
    // other six); `ruby` stands in here as a language this v1 never
    // bundles.
    assert_eq!(run(&mut i, "(treesit-language-available-p 'python)"), "t");
    assert_eq!(run(&mut i, "(treesit-language-available-p 'ruby)"), "nil");
}

#[test]
fn unsupported_language_errors_cleanly() {
    let (mut i, _ed) = setup(RUST_SRC);
    let r = run(&mut i, "(treesit-parser-create 'ruby)");
    assert!(r.starts_with("ERROR"), "expected an error, got {}", r);
}

#[test]
fn parses_real_rust_source_into_a_syntax_tree() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    assert_eq!(run(&mut i, "(treesit-node-type root)"), "\"source_file\"");
    // Two top-level items: the two `fn`s.
    assert_eq!(run(&mut i, "(treesit-node-child-count root)"), "2");
    assert_eq!(
        run(&mut i, "(treesit-node-type (treesit-node-child root 0))"),
        "\"function_item\""
    );
    assert_eq!(
        run(&mut i, "(treesit-node-type (treesit-node-child root 1))"),
        "\"function_item\""
    );
}

#[test]
fn node_positions_and_text_match_the_buffer() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    run(&mut i, "(setq fn1 (treesit-node-child root 0))");
    // `fn add(...` starts at char 1 (the very start of the buffer).
    assert_eq!(run(&mut i, "(treesit-node-start fn1)"), "1");
    // The node's own text, sliced from the actual source, must round-trip
    // through our normal buffer-substring for the same range.
    let node_text = run(&mut i, "(treesit-node-text fn1)");
    let start = run(&mut i, "(treesit-node-start fn1)");
    let end = run(&mut i, "(treesit-node-end fn1)");
    let buf_text = run(&mut i, &format!("(buffer-substring {} {})", start, end));
    assert_eq!(node_text, buf_text);
    assert!(node_text.contains("fn add"), "node text: {}", node_text);
}

#[test]
fn child_parent_roundtrip_and_node_eq() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    run(&mut i, "(setq fn1 (treesit-node-child root 0))");
    run(&mut i, "(setq back (treesit-node-parent fn1))");
    assert_eq!(run(&mut i, "(treesit-node-eq root back)"), "t");
    assert_eq!(run(&mut i, "(treesit-node-parent root)"), "nil");
    assert_eq!(run(&mut i, "(treesit-node-child root 99)"), "nil");
}

#[test]
fn node_at_finds_the_smallest_enclosing_node() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    // Char 4 is the 'a' in "add" (1-based: f=1 n=2 SPC=3 a=4).
    run(&mut i, "(setq n (treesit-node-at 4 p))");
    assert_eq!(run(&mut i, "(treesit-node-type n)"), "\"identifier\"");
    assert_eq!(run(&mut i, "(treesit-node-text n)"), "\"add\"");
}

#[test]
fn query_capture_finds_both_function_names() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    let names = run(
        &mut i,
        "(mapcar (lambda (cap) (treesit-node-text (cdr cap)))
           (treesit-query-capture root \"(function_item name: (identifier) @fn.name)\"))",
    );
    assert_eq!(names, "(\"add\" \"main\")");
    let capture_names = run(
        &mut i,
        "(mapcar #'car
           (treesit-query-capture root \"(function_item name: (identifier) @fn.name)\"))",
    );
    assert_eq!(capture_names, "(fn.name fn.name)");
}

#[test]
fn reparse_reflects_buffer_edits() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-child-count (treesit-parser-root-node p))"
        ),
        "2"
    );
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"fn extra() {}\\n\")");
    // Same parser handle, fresh call: v1 always reparses from the current
    // buffer text (see PLAN.md M12), so this must see the new function.
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-child-count (treesit-parser-root-node p))"
        ),
        "3"
    );
}

// ============================================================
// M108: parse cache correctness (`Buffer::ts_tree`, `treesit::parse`).
// The cache is keyed on `(edit_ticks generation, language)`; every test
// below targets a way that key could be stale or wrong.
// ============================================================

/// Two `treesit-parser-root-node` calls with no edit between them, even
/// through two *separately created* parser handles, must return the
/// SAME underlying tree (`treesit-node-eq` compares `Rc::ptr_eq` on the
/// node's `TsTreeData`, not structural equality -- see
/// `builtins/treesit.rs`) -- this is the actual cache hit, not just
/// "looks the same". The cache lives on the buffer, not the parser
/// object, so a fresh `treesit-parser-create` must still hit it.
#[test]
fn unedited_buffer_reparse_returns_the_same_cached_tree() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p1 (treesit-parser-create 'rust))");
    run(&mut i, "(setq root1 (treesit-parser-root-node p1))");
    run(&mut i, "(setq p2 (treesit-parser-create 'rust))");
    run(&mut i, "(setq root2 (treesit-parser-root-node p2))");
    assert_eq!(
        run(&mut i, "(treesit-node-eq root1 root2)"),
        "t",
        "two parses of the same unedited buffer must share one cached tree"
    );
}

/// An edit invalidates the cache: after inserting text, a fresh
/// `treesit-parser-root-node` call must NOT be `treesit-node-eq` to the
/// pre-edit root, and must observe the edit's effect on the tree shape
/// (this half already existed as `reparse_reflects_buffer_edits` above;
/// this test adds the identity half).
#[test]
fn edited_buffer_reparse_returns_a_different_tree() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root1 (treesit-parser-root-node p))");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"fn extra() {}\\n\")");
    run(&mut i, "(setq root2 (treesit-parser-root-node p))");
    assert_eq!(
        run(&mut i, "(treesit-node-eq root1 root2)"),
        "nil",
        "an edit must invalidate the cached tree"
    );
}

/// Undo bypasses `Buffer::insert`/`delete` (see `undo_step_from`) and is
/// the entry point most likely to be missed by a generation counter --
/// it must still invalidate the cache AND the reparsed tree must
/// reflect the undone state (the function count goes back to 2).
#[test]
fn undo_invalidates_the_cache_and_reflects_the_undone_text() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root1 (treesit-parser-root-node p))");
    assert_eq!(
        run(&mut i, "(treesit-node-child-count root1)"),
        "2",
        "sanity: two functions before the edit"
    );
    // Without this boundary, `undo` would chain back through the setup
    // helper's own initial `insert` too (there is no command loop here
    // to auto-insert boundaries between separate top-level evals -- see
    // other tests' own `(undo-boundary)` calls for the same reason).
    run(&mut i, "(undo-boundary)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"fn extra() {}\\n\")");
    run(&mut i, "(setq root2 (treesit-parser-root-node p))");
    assert_eq!(run(&mut i, "(treesit-node-child-count root2)"), "3");
    run(&mut i, "(undo)");
    run(&mut i, "(setq root3 (treesit-parser-root-node p))");
    assert_eq!(
        run(&mut i, "(treesit-node-eq root2 root3)"),
        "nil",
        "undo must invalidate the cache even though it bypasses insert/delete"
    );
    assert_eq!(
        run(&mut i, "(treesit-node-child-count root3)"),
        "2",
        "the reparsed tree after undo must reflect the undone text, not the pre-undo tree"
    );
}

/// Two parsers for two DIFFERENT languages on the SAME buffer must not
/// hand back each other's cached tree -- the cache key includes the
/// language for exactly this reason (see `Buffer::ts_tree`'s doc).
/// Rust and Elisp grammars both name their root node "source_file" (so
/// that alone isn't a usable discriminator), but they disagree sharply
/// on shape for this source: Rust sees 2 top-level `function_item`s,
/// while Elisp -- fed literal Rust syntax, which is not valid Elisp --
/// recovers into a flat run of ERROR/atom children with a much higher
/// count and no `function_item` type anywhere among them.
#[test]
fn different_languages_on_the_same_buffer_do_not_share_a_cached_tree() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p-rust (treesit-parser-create 'rust))");
    run(&mut i, "(setq root-rust (treesit-parser-root-node p-rust))");
    assert_eq!(run(&mut i, "(treesit-node-child-count root-rust)"), "2");
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-type (treesit-node-child root-rust 0))"
        ),
        "\"function_item\""
    );

    run(&mut i, "(setq p-elisp (treesit-parser-create 'elisp))");
    run(
        &mut i,
        "(setq root-elisp (treesit-parser-root-node p-elisp))",
    );
    assert_ne!(
        run(&mut i, "(treesit-node-child-count root-elisp)"),
        run(&mut i, "(treesit-node-child-count root-rust)"),
        "elisp and rust grammars must not agree on the parse shape for \
         this Rust source -- agreement here would mean the cache handed \
         back the wrong language's tree"
    );
    assert_ne!(
        run(
            &mut i,
            "(treesit-node-type (treesit-node-child root-elisp 0))"
        ),
        "\"function_item\"",
        "elisp's grammar has no function_item node kind at all -- seeing \
         one here would mean this is actually the rust tree"
    );

    // The cache is a single slot per buffer (same shape as
    // `search_snapshot`), so parsing a second language IS allowed to
    // evict the first language's entry -- that's a performance
    // trade-off, not a correctness bug, since one buffer realistically
    // has one active language at a time. What correctness actually
    // requires is what's asserted above: re-fetching after the eviction
    // must reparse fresh under rust's OWN grammar, never silently hand
    // back the elisp tree that's now sitting in the slot.
    run(
        &mut i,
        "(setq root-rust-2 (treesit-parser-root-node p-rust))",
    );
    assert_eq!(
        run(&mut i, "(treesit-node-child-count root-rust-2)"),
        "2",
        "re-fetching rust's tree after an intervening elisp parse must \
         reparse correctly under rust's own grammar, not hand back \
         elisp's tree shape"
    );
    assert_eq!(
        run(
            &mut i,
            "(treesit-node-type (treesit-node-child root-rust-2 0))"
        ),
        "\"function_item\""
    );
}

/// `erase-buffer` + `insert` replaces the entire text (a delete then an
/// insert, both bumping `edit_ticks`); the next parse must reflect the
/// NEW text, not a stale cached tree from before the replacement.
#[test]
fn whole_buffer_replacement_reparses_the_new_text() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root1 (treesit-parser-root-node p))");
    assert_eq!(run(&mut i, "(treesit-node-child-count root1)"), "2");
    run(&mut i, "(erase-buffer)");
    run(&mut i, "(insert \"fn only_one() {}\\n\")");
    run(&mut i, "(setq root2 (treesit-parser-root-node p))");
    assert_eq!(
        run(&mut i, "(treesit-node-eq root1 root2)"),
        "nil",
        "whole-buffer replacement must invalidate the cache"
    );
    assert_eq!(
        run(&mut i, "(treesit-node-child-count root2)"),
        "1",
        "the reparsed tree must reflect the replaced text"
    );
    assert!(run(&mut i, "(treesit-node-text (treesit-node-child root2 0))").contains("only_one"));
}

// ============================================================
// M109: `derive_edit`'s own invariant, independent of tree-sitter.
// ============================================================

/// For every `(old, new)` pair `derive_edit` is asked about, the
/// returned edit (if any) must satisfy
/// `new == old[..start] + new[start..new_end] + old[old_end..]` by
/// construction -- this is the property that makes it structurally
/// impossible to disagree with the text (see the module doc's contrast
/// with a hand-reported `InputEdit`).
fn check_derive_edit_invariant(old: &str, new: &str) {
    match core::treesit::derive_edit(old, new) {
        None => assert_eq!(old, new, "derive_edit returned None for texts that differ"),
        Some(edit) => {
            let mut reconstructed = String::new();
            reconstructed.push_str(&old[..edit.start_byte]);
            reconstructed.push_str(&new[edit.start_byte..edit.new_end_byte]);
            reconstructed.push_str(&old[edit.old_end_byte..]);
            assert_eq!(
                reconstructed, new,
                "derive_edit({:?}, {:?}) => {:?} does not reconstruct `new`",
                old, new, edit
            );
        }
    }
}

/// Like `check_derive_edit_invariant`, but ALSO asserts that
/// `start_byte`, `old_end_byte`, and `new_end_byte` each land on a
/// `char` boundary in whichever string they're used to slice --
/// `start_byte` slices both (`old[..start]`, `new[start..]`), so it
/// must be a boundary in both; `old_end_byte` only ever slices `old`
/// (`old[old_end..]`); `new_end_byte` only ever slices `new`
/// (`new[start..new_end]`). This is deliberately separate from the
/// reconstruction check below: slicing a `str` at a non-boundary index
/// panics, so without this explicit assertion a boundary bug would show
/// up as a test *panic* (the process aborting mid-assertion) rather
/// than a clean assertion failure -- both make the test go red, but
/// conflating them would hide which property actually failed.
fn check_derive_edit_boundaries_and_invariant(old: &str, new: &str) {
    let edit = core::treesit::derive_edit(old, new)
        .unwrap_or_else(|| panic!("derive_edit({:?}, {:?}) returned None", old, new));
    assert!(
        old.is_char_boundary(edit.start_byte) && new.is_char_boundary(edit.start_byte),
        "start_byte {} is not a char boundary in both old {:?} and new {:?}",
        edit.start_byte,
        old,
        new
    );
    assert!(
        old.is_char_boundary(edit.old_end_byte),
        "old_end_byte {} is not a char boundary in old {:?}",
        edit.old_end_byte,
        old
    );
    assert!(
        new.is_char_boundary(edit.new_end_byte),
        "new_end_byte {} is not a char boundary in new {:?}",
        edit.new_end_byte,
        new
    );
    let mut reconstructed = String::new();
    reconstructed.push_str(&old[..edit.start_byte]);
    reconstructed.push_str(&new[edit.start_byte..edit.new_end_byte]);
    reconstructed.push_str(&old[edit.old_end_byte..]);
    assert_eq!(
        reconstructed, new,
        "derive_edit({:?}, {:?}) => {:?} does not reconstruct `new`",
        old, new, edit
    );
}

#[test]
fn derive_edit_satisfies_its_reconstruction_invariant_on_fixed_cases() {
    check_derive_edit_invariant("", "");
    check_derive_edit_invariant("", "abc");
    check_derive_edit_invariant("abc", "");
    check_derive_edit_invariant("abc", "abc");
    check_derive_edit_invariant("abc", "axc");
    check_derive_edit_invariant("hello world", "hello there world");
    check_derive_edit_invariant("hello there world", "hello world");
    check_derive_edit_invariant("abc", "xyz");
    // Multi-byte samples: CJK characters and a combination of ASCII and
    // multi-byte text, including a case where the diff point falls
    // right at a multi-byte character's boundary.
    check_derive_edit_invariant("測試中文", "測試日文");
    check_derive_edit_invariant("a測試b", "a測c試b");
    check_derive_edit_invariant("café", "coffee");
    check_derive_edit_invariant("🎉party", "🎉🎉party");
    check_derive_edit_invariant("prefix共通suffix", "prefix共suffix");

    // The following four target `derive_edit`'s char-boundary retreat
    // specifically (see `check_derive_edit_boundaries_and_invariant`'s
    // doc): each pair is built so the RAW common-prefix or
    // common-suffix scan (before any boundary retreat) stops in the
    // MIDDLE of a multi-byte character, which is exactly the case that
    // needs the retreat loop to do anything at all -- the earlier CJK
    // cases above all happen to stop cleanly on a character boundary
    // already, so they can't tell a working retreat from a deleted one.

    // 1. Common prefix lands mid-character. U+65E5 "日" = E6 97 A5 and
    // U+65E6 "旦" = E6 97 A6 share their first two bytes and differ only
    // in the third, so the raw byte-by-byte prefix scan stops at byte 2
    // of that character (a continuation byte, not a boundary) before
    // retreating back to the start of the character.
    check_derive_edit_boundaries_and_invariant("module 日", "module 旦");

    // 2. Common suffix lands mid-character. U+1F600 "😀" = F0 9F 98 80
    // and U+5F600 (unassigned, but a valid Unicode scalar value) =
    // F1 9F 98 80 share their LAST three bytes and differ only in the
    // first, so the raw byte-by-byte suffix scan (comparing from the
    // end) matches 3 bytes deep into the character -- landing on its
    // second byte, a continuation byte -- before retreating all the way
    // back past the whole character.
    check_derive_edit_boundaries_and_invariant("A\u{1F600}", "B\u{5F600}");

    // 3. Both directions at once, in the same pair: prefix stops
    // mid-character in the leading "日"/"旦" pair from case 1, AND
    // suffix stops mid-character in the trailing emoji pair from case
    // 2, with an unrelated single-byte difference in between.
    check_derive_edit_boundaries_and_invariant("日A\u{1F600}", "旦B\u{5F600}");

    // 4. A 4-byte sequence's boundary case is not just "the same as
    // 3-byte, scaled up": a 4-byte character can need the retreat loop
    // to walk back up to 3 bytes (not 2) to reach its start, which
    // cases 2 and 3 above already exercise but is worth calling out
    // explicitly since the spec asked for at least one emoji-specific
    // case in its own right.
    check_derive_edit_boundaries_and_invariant("x\u{1F600}y", "x\u{5F600}z");
}

#[test]
fn derive_edit_satisfies_its_reconstruction_invariant_on_random_pairs() {
    let mut state: u64 = 0xDEAD_BEEF_1357_2468;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let alphabet = ['a', 'b', 'c', '測', '試', '\n', ' ', '🎉'];
    let rand_string = |next: &mut dyn FnMut() -> u64, max_len: u64| -> String {
        let len = next() % max_len;
        (0..len)
            .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
            .collect()
    };
    for _ in 0..500 {
        let old = rand_string(&mut next, 20);
        let new = rand_string(&mut next, 20);
        check_derive_edit_invariant(&old, &new);
    }
}

// ============================================================
// M109: incremental reparse must produce a tree structurally identical
// to a from-scratch parse of the same text, after every kind of edit --
// see `treesit::derive_edit`'s module doc for why a hand-reported edit
// (the design rejected here) can silently disagree with the text while
// every cheap invariant (has_error, node count, total length) says it's
// fine, which is exactly why this differential test walks the WHOLE
// tree and compares (type start end) triples node-by-node, rather than
// checking any of those cheaper properties.
// ============================================================

/// Recursive Lisp helper: walk `NODE`'s whole subtree and collect an
/// `(equal)`-comparable nested list of `(type start end)` triples, in
/// child order. `treesit-node-start`/`-end` are 1-based CHAR positions
/// (see `builtins/treesit.rs`), not bytes -- but since tree-sitter node
/// boundaries always land on `char` boundaries (grammars never split a
/// multi-byte character), any byte-range disagreement between an
/// incremental and a from-scratch tree also shows up as a char-range
/// disagreement here, so this is exactly as discriminating as comparing
/// raw bytes would be, without needing a new Rust-side test hook.
const TS_DUMP_HELPER: &str = "(defun ts--dump (node)
  (cons (list (treesit-node-type node) (treesit-node-start node) (treesit-node-end node))
        (let ((n (treesit-node-child-count node)) (i 0) (acc nil))
          (while (< i n)
            (push (ts--dump (treesit-node-child node i)) acc)
            (setq i (1+ i)))
          (nreverse acc))))";

/// Assert that `p`'s (incrementally-maintained) current tree is
/// structurally identical to a fresh from-scratch parse of the buffer's
/// current text, under `label` for a failing assertion's context.
fn assert_matches_fresh_parse(i: &mut elisp::Interp, label: &str) {
    let incremental = run(i, "(ts--dump (treesit-parser-root-node p))");
    let fresh = run(
        i,
        "(ts--dump (treesit-parse-string 'verilog (buffer-string)))",
    );
    assert_eq!(
        incremental, fresh,
        "{}: incremental tree diverged from a from-scratch parse",
        label
    );
}

const SV_SRC: &str =
    "module top;\n  logic a;\n  logic b;\n  always_comb begin\n    a = b;\n  end\nendmodule\n";

#[test]
fn incremental_reparse_matches_fresh_parse_across_an_edit_script() {
    let (mut i, _ed) = setup(SV_SRC);
    run(&mut i, TS_DUMP_HELPER);
    run(&mut i, "(setq p (treesit-parser-create 'verilog))");
    assert_matches_fresh_parse(&mut i, "initial parse");

    // Insert a single character.
    run(&mut i, "(goto-char 8)"); // inside "top"
    run(&mut i, "(insert \"x\")");
    assert_matches_fresh_parse(&mut i, "insert a single character");

    // Insert a newline (the `newline-and-indent` path this milestone
    // targets).
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"\\n\")");
    assert_matches_fresh_parse(&mut i, "insert a newline");

    // Paste (insert) multiple lines at once.
    run(&mut i, "(goto-char (point-max))");
    run(
        &mut i,
        "(insert \"  logic c;\\n  logic d;\\n  logic e;\\n\")",
    );
    assert_matches_fresh_parse(&mut i, "insert multiple lines");

    // Delete a block spanning several lines.
    run(&mut i, "(setq del-beg (point))");
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic c;\")");
    run(&mut i, "(goto-char (match-beginning 0))");
    run(&mut i, "(setq blk-beg (point))");
    run(&mut i, "(search-forward \"logic e;\\n\")");
    run(&mut i, "(delete-region blk-beg (point))");
    assert_matches_fresh_parse(&mut i, "delete a multi-line block");

    // kill-line, then yank it back elsewhere.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(search-forward \"logic b;\")");
    run(&mut i, "(goto-char (match-beginning 0))");
    run(&mut i, "(kill-line)");
    assert_matches_fresh_parse(&mut i, "kill-line");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(yank)");
    assert_matches_fresh_parse(&mut i, "yank");

    // undo, then redo (via evil-redo, since there is no separate
    // vanilla `redo` command in this codebase -- see evil.el).
    run(&mut i, "(undo-boundary)");
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"  logic undo_probe;\\n\")");
    assert_matches_fresh_parse(&mut i, "insert before undo");
    run(&mut i, "(undo)");
    assert_matches_fresh_parse(&mut i, "undo");
    run(&mut i, "(evil-redo)");
    assert_matches_fresh_parse(&mut i, "redo");

    // replace-region-contents over a sub-range.
    run(
        &mut i,
        "(replace-region-contents (point-min) (point-max) (concat (buffer-string) \"  logic f;\\n\"))",
    );
    assert_matches_fresh_parse(&mut i, "replace-region-contents");

    // Insert multi-byte CJK text (a comment, so it doesn't need to be
    // valid SystemVerilog to matter for tree shape).
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"  // 測試中文註解\\n\")");
    assert_matches_fresh_parse(&mut i, "insert CJK multi-byte text");

    // Insert at the very start of the buffer (prefix-diff edge case:
    // the common prefix is empty).
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(insert \"// header comment\\n\")");
    assert_matches_fresh_parse(&mut i, "insert at buffer head");

    // Append at the very end (suffix-diff edge case: the common suffix
    // is empty).
    run(&mut i, "(goto-char (point-max))");
    run(&mut i, "(insert \"// trailing comment\\n\")");
    assert_matches_fresh_parse(&mut i, "append at buffer tail");

    // indent-line-to as a no-op (already at the target column) --
    // exercises a call that may not change the text at all.
    run(&mut i, "(goto-char (point-min))");
    run(&mut i, "(indent-line-to 0)");
    assert_matches_fresh_parse(&mut i, "indent-line-to no-op");

    // erase-buffer + insert: whole-buffer replacement.
    run(&mut i, "(erase-buffer)");
    run(&mut i, "(insert \"module only; endmodule\\n\")");
    assert_matches_fresh_parse(&mut i, "erase-buffer + insert");
}

/// A random (fixed-seed) sequence of small edits on a modest buffer,
/// each one re-checked against a from-scratch parse. This is the fuzz
/// pass called for in the spec: unlike the scripted test above, it
/// isn't hand-picked to hit only the cases we thought of.
#[test]
fn incremental_reparse_survives_a_random_edit_sequence() {
    let (mut i, _ed) = setup(SV_SRC);
    run(&mut i, TS_DUMP_HELPER);
    run(&mut i, "(setq p (treesit-parser-create 'verilog))");
    assert_matches_fresh_parse(&mut i, "initial parse");

    let mut state: u64 = 0x00C0_FFEE_1234_5678;
    let mut next = move || {
        // xorshift64 -- deterministic, no external RNG dependency.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let snippets = [
        "x",
        "\n",
        "logic q;\n",
        "  // 測試\n",
        "always_comb begin\n  a = b;\nend\n",
        "",
    ];

    for step in 0..200 {
        let choice = (next() % 4) as u32;
        let len = run(&mut i, "(point-max)").parse::<i64>().unwrap_or(1);
        match choice {
            0 => {
                // Insert a snippet at a random position.
                let pos = 1 + (next() % (len as u64).max(1)) as i64;
                let snippet = snippets[(next() % snippets.len() as u64) as usize];
                run(&mut i, &format!("(goto-char {})", pos));
                run(&mut i, &format!("(insert {:?})", snippet));
            }
            1 => {
                // Delete a small random region.
                if len > 2 {
                    let a = 1 + (next() % (len as u64 - 1)) as i64;
                    let b = (a + 1 + (next() % 5) as i64).min(len);
                    if a < b {
                        run(&mut i, &format!("(delete-region {} {})", a, b));
                    }
                }
            }
            2 => {
                run(&mut i, "(undo-boundary)");
            }
            _ => {
                if run(&mut i, "(undo)").starts_with("ERROR") {
                    // Nothing left to undo -- fine, just skip.
                }
            }
        }
        assert_matches_fresh_parse(&mut i, &format!("random step {}", step));
    }
}

#[test]
fn treesit_fontify_region_applies_faces_via_overlays() {
    let (mut i, _ed) = setup(RUST_SRC);
    run(&mut i, "(setq p (treesit-parser-create 'rust))");
    run(&mut i, "(setq root (treesit-parser-root-node p))");
    run(
        &mut i,
        "(treesit-fontify-region root '((\"(function_item name: (identifier) @fn.name)\" . font-lock-function-name-face)))",
    );
    let faces = run(
        &mut i,
        "(let (faces)
           (dolist (ov (overlays-in (point-min) (point-max)) faces)
             (when (overlay-get ov 'treesit-face)
               (push (overlay-get ov 'face) faces))))",
    );
    assert_eq!(
        faces,
        "(font-lock-function-name-face font-lock-function-name-face)"
    );
}
