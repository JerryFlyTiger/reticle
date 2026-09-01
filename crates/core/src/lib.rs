// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

pub mod buffer;
pub mod builtins;
pub mod commands;
pub mod complete;
pub mod dired;
pub mod editor;
pub mod gapbuffer;
pub mod highlight;
pub mod keymap;
pub mod panel;
pub mod redisplay;
pub mod remote;
pub mod sexp;
pub mod textdiff;
pub mod treesit;

use std::cell::RefCell;
use std::rc::Rc;

use elisp::{Interp, Value};

pub const SIMPLE_EL: &str = include_str!("../lisp/simple.el");
pub const MODES_EL: &str = include_str!("../lisp/modes.el");
pub const INDENT_EL: &str = include_str!("../lisp/indent.el");
pub const ELECTRIC_PAIR_EL: &str = include_str!("../lisp/electric-pair.el");
pub const EVIL_EL: &str = include_str!("../lisp/evil.el");
pub const DABBREV_EL: &str = include_str!("../lisp/dabbrev.el");
pub const ORG_EL: &str = include_str!("../lisp/org.el");
pub const TREESIT_EL: &str = include_str!("../lisp/treesit.el");
pub const LSP_EL: &str = include_str!("../lisp/lsp.el");
pub const THEMES_EL: &str = include_str!("../lisp/themes.el");
pub const IELM_EL: &str = include_str!("../lisp/ielm.el");
pub const DIRED_EL: &str = include_str!("../lisp/dired.el");
pub const ESHELL_EL: &str = include_str!("../lisp/eshell.el");
pub const SHELL_COMMAND_EL: &str = include_str!("../lisp/shell-command.el");
pub const COMPILE_EL: &str = include_str!("../lisp/compile.el");
pub const SEARCH_EL: &str = include_str!("../lisp/search.el");
pub const VERILOG_AUTO_EL: &str = include_str!("../lisp/verilog-auto.el");
pub const VERILOG_COMPLETE_EL: &str = include_str!("../lisp/verilog-complete.el");
pub const VERILOG_NAV_EL: &str = include_str!("../lisp/verilog-nav.el");

/// Create the editor, install it into the interpreter, register all
/// editing builtins, and load the elisp command layer.
pub fn init_editor(interp: &mut Interp) -> Rc<RefCell<editor::Editor>> {
    let ed = Rc::new(RefCell::new(editor::Editor::new()));
    ed.borrow_mut().global_keymap = keymap::make_keymap();
    interp.ext = Some(ed.clone() as Rc<dyn std::any::Any>);
    builtins::register_all(interp);

    // Symbols the redisplay engine looks up with intern_soft.
    interp.intern("invisible");
    interp.intern("face");

    // Route `message` output to the echo area instead of stdout.
    {
        let ed = ed.clone();
        interp.output = Some(Box::new(move |text: &str| {
            let trimmed = text.trim_end_matches('\n');
            if !trimmed.is_empty() {
                ed.borrow_mut().echo = Some(trimmed.to_string());
            }
        }));
    }

    // GC root provider: every persistent Value the editor holds outside
    // the symbol table. Uses a Weak so the provider doesn't keep the
    // editor alive, and try_borrow so a collection attempted while some
    // command holds the editor mutably aborts (incomplete roots would
    // be unsafe) rather than panicking.
    //
    // M52: this only covers buffers/keymaps reachable *from the editor
    // itself* (editor.buffers, editor.windows, global_keymap, ...).
    // Values reachable only *through* an elisp Value — a keymap's key
    // bindings, an overlay's props after delete-overlay, a killed
    // buffer's locals still referenced by a variable — are covered by
    // each type's `ExtTracer` (see `keymap::trace_keymap`,
    // `buffer::trace_overlay`, `editor::trace_buffer`), which `gc::mark`
    // invokes when it walks into a `Value::Ext`. The two are
    // complementary and safely overlap: marking is `visited`-gated, so
    // emitting the same value from both paths is a no-op the second time.
    {
        let weak_ed = Rc::downgrade(&ed);
        interp
            .gc_roots
            .push(Box::new(move |emit: &mut dyn FnMut(&Value)| {
                let Some(ed) = weak_ed.upgrade() else {
                    return true;
                };
                let Ok(editor) = ed.try_borrow() else {
                    return false;
                };
                for buf in &editor.buffers {
                    let Ok(b) = buf.try_borrow() else {
                        return false;
                    };
                    emit(&b.keymap);
                    emit(&b.major_mode);
                    for v in b.locals.values() {
                        emit(v);
                    }
                    for ov in &b.overlays {
                        let Ok(o) = ov.try_borrow() else { return false };
                        for (_, v) in &o.props {
                            emit(v);
                        }
                    }
                }
                for win in editor.windows.values() {
                    // Normally in editor.buffers already; enumerated anyway
                    // in case a kill-buffer path ever leaves a straggler.
                    let Ok(b) = win.buffer.try_borrow() else {
                        return false;
                    };
                    emit(&b.keymap);
                    emit(&b.major_mode);
                }
                emit(&editor.global_keymap);
                emit(&editor.last_command);
                emit(&editor.this_command);
                // M28: a pending `capture-next-key` callback (often a fresh
                // anonymous lambda) is reachable only from here between the
                // call that armed it and the key event that fires it.
                if let Some(v) = &editor.key_capture {
                    emit(v);
                }
                for v in editor.swapped_globals.values().flatten() {
                    emit(v);
                }
                if let Some(mb) = &editor.minibuffer {
                    emit(&mb.pending.command);
                    for v in &mb.pending.collected {
                        emit(v);
                    }
                }
                true
            }));
    }

    if let Err(flow) = interp.eval_source(SIMPLE_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading simple.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(MODES_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading modes.el: {}", msg);
    }
    // M36: the indentation engine (TAB/RET auto-indent). Loaded right
    // after modes.el (extends `treesit--prog-mode-setup', and needs
    // `prog-mode-hook') and before evil.el (whose `evil-open-below'/
    // `evil-open-above' call `indent-current-line-if-supported') -- see
    // indent.el's own header for the full design.
    if let Err(flow) = interp.eval_source(INDENT_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading indent.el: {}", msg);
    }
    // M37: electric-pair-mode (auto-close/skip matching brackets and
    // quotes). Loaded right after indent.el -- needs `prog-mode-hook'
    // (modes.el) the same way indent.el does, and its own `prog-mode-
    // hook' addition should see the buffer already past indent.el's
    // setup, though nothing here actually depends on indent.el itself --
    // see electric-pair.el's own header for why this hangs off
    // `post-self-insert-hook' rather than a keybinding, and why that
    // makes it safe ahead of evil.el's `inhibit-self-insert' (loaded
    // next) rather than needing to coordinate with it.
    if let Err(flow) = interp.eval_source(ELECTRIC_PAIR_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading electric-pair.el: {}", msg);
    }
    // M29: evil-mode's keymap layer. Loaded right after modes.el (needs
    // major-mode-internal-get/auto-mode-alist's notion of major modes for
    // its emacs-state-modes rule) and before org.el/treesit.el (no
    // dependency either way; just keeps the "foundational, cross-mode
    // layers first" ordering already established by modes.el's own
    // header comment).
    if let Err(flow) = interp.eval_source(EVIL_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading evil.el: {}", msg);
    }
    // M31: dabbrev completion engine (dabbrev-expand/M-/, plus the
    // C-n/C-p commands evil.el's own insert-map binds to it). Load
    // order relative to evil.el doesn't matter -- see evil.el's own
    // M31 section -- placed here simply to keep it textually next to
    // the insert-state layer it extends.
    if let Err(flow) = interp.eval_source(DABBREV_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading dabbrev.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(ORG_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading org.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(TREESIT_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading treesit.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(LSP_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading lsp.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(THEMES_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading themes.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(IELM_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading ielm.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(DIRED_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading dired.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(ESHELL_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading eshell.el: {}", msg);
    }
    if let Err(flow) = interp.eval_source(SHELL_COMMAND_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading shell-command.el: {}", msg);
    }
    // M80: compile/recompile + next-error/previous-error. Loaded right
    // after shell-command.el -- reuses two of its buffer-parametrized
    // helpers (`shell-command--insert-output'/`shell-command--maybe-
    // show'), see compile.el's own header for exactly which and why.
    if let Err(flow) = interp.eval_source(COMPILE_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading compile.el: {}", msg);
    }
    // M82: search-project/search-again, streamed into `*search*'. Loaded
    // right after compile.el -- reuses the same two shell-command.el
    // helpers compile.el does (`shell-command--insert-output' indirectly
    // via its own `search--insert-line', `shell-command--maybe-show'),
    // and `shell-command--default-dir' as a fallback when the current
    // buffer has no file -- see search.el's own header for exactly why.
    if let Err(flow) = interp.eval_source(SEARCH_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading search.el: {}", msg);
    }
    // M39: verilog-mode's AUTOINST/AUTOWIRE/AUTOARG core. Loaded last:
    // its only real dependencies are modes.el (verilog-mode/
    // verilog-mode-hook) and indent.el (standard-indent-width), both
    // satisfied long before this point, and no other file here depends
    // on anything IN it -- see verilog-auto.el's own header.
    if let Err(flow) = interp.eval_source(VERILOG_AUTO_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading verilog-auto.el: {}", msg);
    }
    // M54: Verilog port-name completion (`local-completion-function',
    // wired up ahead of LSP/dabbrev by `completion-at-point' in
    // lsp.el). Loaded last of all, after both of its own dependencies:
    // `verilog-auto.el' just above (reuses several of its pure
    // module/port-lookup helpers -- see verilog-complete.el's own
    // header for exactly which, and why NOT `--module-ports' itself)
    // and `lsp.el' (defines `local-completion-function' as a plain
    // `defvar', and `modes.el' -- loaded long before either -- only
    // references `verilog-complete-at-point' by NAME inside a hook
    // closure that doesn't run until a `verilog-mode' buffer is set up,
    // well after every file here has finished loading, so no load-order
    // constraint from that direction either).
    if let Err(flow) = interp.eval_source(VERILOG_COMPLETE_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading verilog-complete.el: {}", msg);
    }
    // M55: Verilog cross-file module-definition jump (`local-definition-
    // function', wired up ahead of LSP by `lsp-definition-at-point' in
    // lsp.el). Loaded last of all, for the same reasons as
    // verilog-complete.el just above: reuses several of verilog-auto.el's
    // pure module-lookup helpers (see verilog-nav.el's own header), and
    // `lsp.el'/`modes.el' only reference its entry point by NAME inside a
    // `defvar' default / hook closure, neither of which runs until well
    // after every file here has finished loading.
    if let Err(flow) = interp.eval_source(VERILOG_NAV_EL) {
        let msg = interp.describe_flow(&flow);
        panic!("error loading verilog-nav.el: {}", msg);
    }
    // Ship the editor's own lisp layer byte-compiled, like GNU Emacs's
    // .elc files. User init.el functions are left interpreted and tier
    // up automatically once they get hot.
    elisp::compiler::compile_all_defined(interp);
    ed
}

/// After this much quiet, the idle tick runs the M9 idle GC path.
const IDLE_GC_AFTER: std::time::Duration = std::time::Duration::from_secs(2);

/// The one background-housekeeping entry point (M15), shared by both
/// frontends: call it from the event loop whenever there is no input to
/// process — each poll timeout in the TUI, each frame in the GUI.
/// `quiet_for` is the time since the last user input.
///
/// It pumps async LSP clients (delivering `lsp-request-async` callbacks
/// and folding in diagnostics; a cheap no-op with no clients connected)
/// and drives the M9 GC — pressure check while the user is active, idle
/// collection once they've been quiet a while (self-limiting: the
/// registered-since-GC floor makes repeated idle calls free).
pub fn idle_tick(interp: &mut Interp, quiet_for: std::time::Duration) {
    let _ = interp.eval_source("(lsp-process-pending-all)");
    let _ = interp.eval_source("(eshell-process-pending-all)");
    // M79: M-!/M-&/M-|'s own pump, independent of eshell's (see
    // shell-command.el's header comment for why the two don't share a
    // list or a pump function).
    let _ = interp.eval_source("(shell-command-process-pending-all)");
    // M80: compile/recompile's own pump, independent of shell-command's
    // (see compile.el's header comment for why the two don't share a
    // list or a pump function).
    let _ = interp.eval_source("(compile-process-pending-all)");
    // M82: search-project/search-again's own pump, independent of
    // compile's/shell-command's (see search.el's header comment for why
    // the three don't share a list or a pump function).
    let _ = interp.eval_source("(search-process-pending-all)");
    let ed = editor::editor(interp);
    highlight::tick(interp, &ed);
    drain_background_output(interp, &ed);
    elisp::gc::maybe_auto_collect(interp, quiet_for >= IDLE_GC_AFTER);
    // M51: drive the documentHighlight auto-trigger (`lsp-idle-highlight-
    // delay-ms`). The threshold comparison itself lives in elisp (so
    // it's user-tunable); Rust's only job is converting the Duration to
    // an integer of milliseconds elisp can hold. Clamped to `u32::MAX`
    // (~49.7 days) rather than left unbounded, so an already-huge
    // `quiet_for` (elisp integers are i64, but `Duration::as_millis`
    // returns u128) can't overflow what gets spliced into the elisp
    // source string; a threshold in the tens-to-hundreds-of-ms range
    // will have long since fired well before that ceiling matters. See
    // `lsp-idle-highlight-delay-ms`'s docstring for the user-visible
    // consequence of setting the threshold above this ceiling.
    let quiet_ms = quiet_for.as_millis().min(u32::MAX as u128) as u64;
    let _ = interp.eval_source(&format!("(lsp--idle-highlight-tick {quiet_ms})"));
}

/// Cap on `*background-output*`'s own line count, independent of
/// `elisp::bglog`'s 500-line in-memory bound. `bglog` only guarantees
/// its own queue is bounded between drains; every drain this function
/// runs *appends* to the buffer, and a session runs many drains (M60's
/// whole motivation is a source that logs continuously), so without a
/// second cap here the bound just moves one hop downstream instead of
/// actually existing. Sized generously above `bglog`'s per-drain cap
/// (worth keeping more history visible in a buffer than in the transient
/// collector) but still bounded, so a chatty source across a long
/// session can't grow this buffer without limit.
///
/// `pub` so `crates/core/tests/background_output_tests.rs` can assert
/// against the real constant instead of a copy-pasted literal that would
/// silently stop meaning anything the day this number changes.
///
/// Three known trade-offs of this trim-on-drain design, left as-is
/// (M60 review, round 3):
///
/// 1. **Cost while chattering near the cap.** `Buffer::search_text()`'s
///    cache is keyed on `edit_ticks`, and the `edit_insert` loop above
///    always bumps it, so every idle tick that drains anything pays a
///    full `to_string()` over the whole buffer to even *check* whether a
///    trim is needed -- and if a trim does fire, `GapBuffer::move_gap`
///    walks the gap from the tail (where appends leave it) to position 0
///    and the next append walks it back. So a buffer sitting at the cap
///    with a continuously noisy source pays roughly two to three O(cap)
///    memory moves per tick. Estimated sub-millisecond at this cap size
///    and considered acceptable; this is a deliberate
///    simplicity-over-throughput choice, not an oversight.
/// 2. **The buffer is not `read_only`.** A user who switches to
///    `*background-output*` and types into it can have their own text
///    silently removed if it ends up inside the next trim's `[0, cut)`
///    range -- not a panic (`Buffer::adjust_positions_delete` clamps
///    `point` like any other delete), just content vanishing with no
///    warning. Deliberately left editable rather than adding
///    `read_only` handling that no other M60 file touches.
/// 3. **Non-selected windows' cached `point` can go stale.** Both
///    `editor::edit_insert` and `editor::edit_delete` adjust
///    `Buffer.point` and every window's `window_start`, but not a
///    non-selected `Window`'s own cached `point` field. This is a
///    pre-existing limitation of that machinery, not something M60
///    introduced -- but M60 is the first caller that runs
///    non-user-triggered edits repeatedly against a buffer the user might
///    actually be looking at, so it makes this existing gap easier to
///    hit than before.
pub const MAX_BACKGROUND_OUTPUT_LINES: usize = 5000;

/// M60: pull whatever `elisp::bglog` has accumulated (LSP/worker stderr,
/// highlight-worker diagnostics -- see that module's doc) into an
/// on-demand `*background-output*` buffer, so it's visible somewhere
/// other than a diff-based `Grid` that would never revisit it (see
/// `crates/elisp/src/bglog.rs`'s module doc for the motivating bug).
/// Silent when there's nothing new; the first time a tick actually
/// drains something, one `message` announces the buffer's existence
/// (`Editor::background_output_notified` latches this so a chatty
/// source doesn't spam the echo area every tick after).
fn drain_background_output(interp: &mut Interp, ed: &Rc<RefCell<editor::Editor>>) {
    let lines = elisp::bglog::drain();
    if lines.is_empty() {
        return;
    }
    let buf = {
        let mut ed_mut = ed.borrow_mut();
        match ed_mut.find_buffer("*background-output*") {
            Some(b) => b,
            None => {
                let b = Rc::new(RefCell::new(buffer::Buffer::new("*background-output*", "")));
                ed_mut.buffers.push(b.clone());
                b
            }
        }
    };
    {
        // Goes through `editor::edit_insert` (not `Buffer::insert`
        // directly) so any window currently showing this buffer keeps
        // its `window_start` correctly adjusted, same as every other
        // programmatic insert in this crate -- today the insert point is
        // always the buffer's end so it happens to be equivalent either
        // way, but that's incidental, not a contract this function
        // should rely on.
        let mut pos = buf.borrow().text.len();
        for line in &lines {
            let mut with_nl = line.clone();
            with_nl.push('\n');
            pos += editor::edit_insert(ed, &buf, pos, &with_nl);
        }
    }
    {
        // Enforce MAX_BACKGROUND_OUTPUT_LINES by trimming from the
        // front. Every append above ends in exactly one '\n' and this
        // buffer is never edited any other way, so "newline count" and
        // "line count" coincide -- counting newlines in a full-text
        // snapshot is the simplest way to find the cut point.
        let b = buf.borrow_mut();
        let text = b.search_text();
        let total_lines = text.matches('\n').count();
        if total_lines > MAX_BACKGROUND_OUTPUT_LINES {
            let excess = total_lines - MAX_BACKGROUND_OUTPUT_LINES;
            let mut newlines_seen = 0usize;
            let mut cut_at = None;
            for (char_idx, c) in text.chars().enumerate() {
                if c == '\n' {
                    newlines_seen += 1;
                    if newlines_seen == excess {
                        cut_at = Some(char_idx + 1);
                        break;
                    }
                }
            }
            drop(text);
            if let Some(cut) = cut_at {
                drop(b);
                editor::edit_delete(ed, &buf, 0, cut);
            }
        }
    }
    let already_notified = ed.borrow().background_output_notified;
    if !already_notified {
        ed.borrow_mut().background_output_notified = true;
        let msg = interp.intern("message");
        let _ = elisp::eval::apply_function(
            interp,
            &Value::Sym(msg),
            vec![Value::string(
                "Background output captured in *background-output*".to_string(),
            )]
            .into(),
        );
    }
}

/// Whether any async work source (currently: live LSP clients) exists —
/// frontends use this to pick a fast poll cadence only when something
/// could actually deliver results (the GUI shouldn't wake 10×/second
/// forever just in case).
pub fn has_async_work(interp: &mut Interp) -> bool {
    let clients = interp.intern("lsp--clients");
    if interp
        .sym_value(clients)
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        return true;
    }
    // A running eshell command also needs the fast poll cadence.
    let procs = interp.intern("eshell--procs");
    if interp.sym_value(procs).map(|v| v.truthy()).unwrap_or(false) {
        return true;
    }
    // M79 left a gap here: `shell-command--procs' (M-!/M-&/M-|'s own job
    // list, shell-command.el) was never checked, so the frontend never
    // switched to the fast poll cadence for a running shell-command job
    // -- caught while wiring up M80's `compile--procs' alongside it.
    let shell_command_procs = interp.intern("shell-command--procs");
    if interp
        .sym_value(shell_command_procs)
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        return true;
    }
    // M80: `compile'/`recompile's own job list.
    let compile_procs = interp.intern("compile--procs");
    if interp
        .sym_value(compile_procs)
        .map(|v| v.truthy())
        .unwrap_or(false)
    {
        return true;
    }
    // M82: `search-project'/`search-again's own job list -- omitting
    // this check would leave the frontend on the slow poll cadence
    // while a search is still streaming, the exact gap M80's own
    // comment above documents having caught for `shell-command--procs'.
    let search_procs = interp.intern("search--procs");
    interp
        .sym_value(search_procs)
        .map(|v| v.truthy())
        .unwrap_or(false)
}

/// Load the user's init file from `dir` (adds it to load-path, sets
/// user-init-file / user-emacs-directory). Errors land in the echo area
/// instead of aborting startup, like Emacs.
pub fn load_init_from(interp: &mut Interp, ed: &Rc<RefCell<editor::Editor>>, dir: &str) {
    interp.load_path.push(dir.to_string());
    let init = format!("{}/init.el", dir);
    let uif = interp.intern("user-init-file");
    interp.symbols[uif as usize].special = true;
    interp.set_sym_value(uif, elisp::Value::string(init.clone()));
    let ued = interp.intern("user-emacs-directory");
    interp.symbols[ued as usize].special = true;
    interp.set_sym_value(ued, elisp::Value::string(format!("{}/", dir)));
    if std::path::Path::new(&init).is_file() {
        if let Err(flow) = interp.load_file(&init) {
            let msg = interp.describe_flow(&flow);
            ed.borrow_mut().echo = Some(format!("Error loading init file: {}", msg));
        }
    }
}

/// Standard startup: ~/.reticle/init.el.
pub fn load_user_init(interp: &mut Interp, ed: &Rc<RefCell<editor::Editor>>) {
    if let Ok(home) = std::env::var("HOME") {
        load_init_from(interp, ed, &format!("{}/.reticle", home));
    }
}
