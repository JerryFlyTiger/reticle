// SPDX-License-Identifier: LicenseRef-FSL-1.1-ALv2
// Copyright 2026 Jerry Chen
//
// Reticle is source-available software, licensed under the Functional
// Source License 1.1 with an Apache 2.0 future grant. It is not open source.
// See LICENSE.md for the terms, and THIRD_PARTY_LICENSES.md for the licenses
// of the dependencies it links against.

mod diff;

use std::cell::RefCell;
use std::io::{BufWriter, Write};
use std::rc::Rc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::{cursor, execute, terminal};

use core::commands::{handle_key, Key};
use core::editor::Editor;
use core::keymap::{ctrl_encode, META};
use core::redisplay::{render, Grid};
use elisp::{Interp, Value};

use diff::diff_draw;

/// Restores the terminal even if we panic mid-session.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        // Reset any DECSCUSR cursor-shape override (M28 `cursor-shape`,
        // renamed to the GNU-named `cursor-type` in M32) before leaving
        // raw mode, so a custom bar/box cursor doesn't leak into the
        // user's shell after reticle exits.
        let _ = write!(out, "\x1b[0 q");
        let _ = execute!(out, cursor::Show, terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
    }
}

pub fn run_tui(interp: &mut Interp, ed: &Rc<RefCell<Editor>>) -> std::io::Result<()> {
    terminal::enable_raw_mode()?;
    let _guard = TerminalGuard;
    let mut out = BufWriter::new(std::io::stdout());
    execute!(out, terminal::EnterAlternateScreen, cursor::Hide)?;

    let (cols, rows) = terminal::size()?;
    ed.borrow_mut().frame = (cols as usize, rows as usize);

    // The unified idle tick (M15) runs on every poll timeout: it pumps
    // async LSP results and drives the M9 GC (pressure check while
    // active, idle collection after ~2s quiet). Poll faster while async
    // work could actually deliver something, so callbacks land promptly.
    let mut last_input = std::time::Instant::now();

    // Previous frame (M23a): `draw` diffs against this and only writes
    // the terminal cells that actually changed. `None` forces a full
    // redraw — true for the first frame, and again after a resize since
    // the old frame's coordinates no longer line up with the new size.
    let mut prev_grid: Option<Grid> = None;
    // M28 `cursor-shape` (renamed `cursor-type` in M32): the last
    // DECSCUSR escape actually written, so `draw` only re-sends it when
    // the wanted shape changes (the same "don't touch the terminal
    // unless something changed" discipline `prev_grid` gives cell
    // content).
    let mut prev_cursor_shape: Option<&'static str> = None;
    // M88: true once the first real frame has been drawn -- gates
    // `core::frontend_started`, called exactly once below, which in turn
    // is what lets `lsp--autostart-tick` (part of the idle tick just
    // below) actually spawn anything. See `frontend_started`'s own doc.
    let mut frontend_started = false;

    loop {
        draw(interp, ed, &mut out, &mut prev_grid, &mut prev_cursor_shape)?;
        if !frontend_started {
            core::frontend_started(interp);
            frontend_started = true;
        }
        if ed.borrow().quit {
            break;
        }
        let poll_ms = if core::has_async_work(interp) {
            100
        } else {
            500
        };
        if !crossterm::event::poll(std::time::Duration::from_millis(poll_ms))? {
            core::idle_tick(interp, last_input.elapsed());
            continue;
        }
        match crossterm::event::read()? {
            Event::Key(kev) => {
                if kev.kind == crossterm::event::KeyEventKind::Press
                    || kev.kind == crossterm::event::KeyEventKind::Repeat
                {
                    if let Some(key) = convert_key(kev) {
                        handle_key(interp, ed, key);
                        last_input = std::time::Instant::now();
                        // Pressure-triggered collection between keys;
                        // no-op unless gc-cons-threshold was exceeded.
                        elisp::gc::maybe_auto_collect(interp, false);
                    }
                }
            }
            Event::Resize(c, r) => {
                ed.borrow_mut().frame = (c as usize, r as usize);
                prev_grid = None;
            }
            _ => {}
        }
        if ed.borrow().quit {
            draw(interp, ed, &mut out, &mut prev_grid, &mut prev_cursor_shape)?;
            break;
        }
    }
    Ok(())
}

fn convert_key(kev: KeyEvent) -> Option<Key> {
    let ctrl = kev.modifiers.contains(KeyModifiers::CONTROL);
    let meta = kev.modifiers.contains(KeyModifiers::ALT);
    let base: Key = match kev.code {
        KeyCode::Char(c) => {
            // crossterm's unix raw-byte parser (crossterm 0.28
            // `src/event/sys/unix/parse.rs:110-113`) maps the C0 control
            // bytes 0x1C..=0x1F to `KeyCode::Char('4'..='7') +
            // KeyModifiers::CONTROL` via `c - 0x1C + b'4'`, because on a
            // terminal that reports raw C0 bytes (traditional tty
            // behaviour, still the common case), `Ctrl+4`..`Ctrl+7` and
            // `Ctrl+\`/`Ctrl+]`/`Ctrl+^`/`Ctrl+_` are physically
            // indistinguishable — the tty sends the same byte either
            // way (GNU Emacs treats them as the same key in a terminal
            // too). If we ran `c` through `ctrl_encode` here we'd land
            // on `ctrl_encode('7') = 0x37 | CTRL`, which matches neither
            // `"C-/"` (0x2f | CTRL) nor `"C-_"` (31) in simple.el's undo
            // binding, so `C-/` and `C-_` were both silently
            // unreachable in the TUI. Undo the crossterm remap and
            // recover the original C0 byte instead; `"C-4"` becomes
            // unreachable as a *side effect*, but that's not a real
            // loss since a raw-C0 terminal can't tell it apart from
            // `C-\` anyway.
            //
            // This does NOT hold for every terminal, only for ones
            // reporting raw C0 bytes. Any terminal sending
            // `CSI <codepoint>;<mods>u` (Fixterms/"CSI u", the protocol
            // family kitty's keyboard enhancement is part of —
            // crossterm's own `parse_csi_u_encoded_key_code`,
            // `event/sys/unix/parse.rs:203`/`497`) reports Ctrl+4..7
            // already disambiguated from Ctrl+\]^_, and *some terminals
            // send CSI u on their own* (e.g. iTerm2's "Report modifiers
            // using CSI u") without this app negotiating
            // `PushKeyboardEnhancementFlags` (which `run_tui` above
            // never does either). `KeyEvent` has no field this code can
            // use to tell which path produced a given `Char('4'..'7')`
            // + CONTROL event — `state` is only populated once the app
            // has opted into `DISAMBIGUATE_ESCAPE_CODES`. This remap is
            // harmless today because no `.el` file in this repo binds
            // `C-4`..`C-7`; if one ever does, or if this file starts
            // pushing keyboard enhancement flags, this branch needs to
            // be revisited — e.g. only applying the remap when
            // `state` shows the event was *not* disambiguated.
            //
            // Also unix-only structurally, via `cfg!(unix)` rather than
            // `#[cfg(unix)]` (so both branches keep compiling on every
            // platform, with no dead-code warnings on non-unix): on
            // crossterm's Windows backend (`event/sys/windows/parse.rs
            // :260-283`, via `ToUnicodeEx`), `Char('4') + CONTROL`
            // really is Ctrl+4, distinguishable from `Char('\\') +
            // CONTROL`; applying this remap there would wrongly
            // collapse a correctly-disambiguated event into `C-\`.
            let mut code = if cfg!(unix) && ctrl && ('4'..='7').contains(&c) {
                0x1C + (c as u8 - b'4') as i64
            } else if ctrl {
                ctrl_encode(c)
            } else {
                c as i64
            };
            if meta {
                code |= META;
            }
            return Some(Key::Char(code));
        }
        KeyCode::Enter => Key::Char(13),
        KeyCode::Tab => Key::Char(9),
        KeyCode::Backspace => Key::Char(127),
        KeyCode::Esc => Key::Char(27),
        KeyCode::Up => Key::Sym("up".into()),
        KeyCode::Down => Key::Sym("down".into()),
        KeyCode::Left => Key::Sym("left".into()),
        KeyCode::Right => Key::Sym("right".into()),
        KeyCode::Home => Key::Sym("home".into()),
        KeyCode::End => Key::Sym("end".into()),
        KeyCode::PageUp => Key::Sym("prior".into()),
        KeyCode::PageDown => Key::Sym("next".into()),
        KeyCode::Delete => Key::Sym("deletechar".into()),
        _ => return None,
    };
    // Apply modifiers to non-char keys the Emacs way (M-<up> etc. later);
    // for now meta on special keys is dropped.
    if let (Key::Char(code), true) = (&base, meta) {
        return Some(Key::Char(code | META));
    }
    Some(base)
}

/// Renders the current editor state and writes only what changed since
/// `prev_grid` to `out` (M23a — see `diff::diff_draw`), plus (M28,
/// migrated to `cursor-type` in M32) any change to the DECSCUSR cursor
/// escape. Idle poll timeouts with no visible change therefore touch
/// the terminal not at all: no escape sequences, no flush.
fn draw(
    interp: &Interp,
    ed: &Rc<RefCell<Editor>>,
    out: &mut impl Write,
    prev_grid: &mut Option<Grid>,
    prev_cursor_shape: &mut Option<&'static str>,
) -> std::io::Result<()> {
    let grid = render(interp, ed);
    let mut wrote = diff_draw(out, prev_grid.as_ref(), &grid)?;
    let shape = cursor_type_escape(interp);
    if emit_cursor_shape_if_changed(out, *prev_cursor_shape, shape)? {
        wrote = true;
    }
    *prev_cursor_shape = shape;
    if wrote {
        out.flush()?;
    }
    *prev_grid = Some(grid);
    Ok(())
}

/// The DECSCUSR escape for the elisp `cursor-type` variable ('box /
/// 'bar / 'hbar — GNU Emacs's own names; M32 migration from the
/// self-made `cursor-shape', which spelled the box value 'block), or
/// `None` for "leave the terminal's cursor alone" — nil (the default),
/// unset, or any other value.
fn cursor_type_escape(interp: &Interp) -> Option<&'static str> {
    let sym = interp.intern_soft("cursor-type")?;
    match interp.sym_value(sym)? {
        Value::Sym(id) => match interp.sym_name(id) {
            "box" => Some("\x1b[2 q"),
            "bar" => Some("\x1b[6 q"),
            "hbar" => Some("\x1b[4 q"),
            _ => None,
        },
        _ => None,
    }
}

/// Writes the escape that takes the terminal cursor from `prev` to
/// `wanted`, iff they differ — the same "only touch the terminal on an
/// actual change" discipline `diff_draw` applies to cell content.
/// `wanted = None` with a previously-set shape emits a DECSCUSR reset,
/// so e.g. `(setq cursor-type nil)` restores the terminal's default
/// cursor mid-session instead of leaving the last box/bar stuck until
/// exit (evil-mode turning off used to reach this same path pre-M32;
/// it now sets a concrete `'box` instead — see evil.el's `evil-mode` —
/// so this reset is reachable from a direct `cursor-type` override, not
/// from evil-mode's own off-switch, but the mechanism is unchanged). A
/// pure function over `impl Write`, like `diff_draw`, so it's
/// unit-testable without a real terminal or interpreter — see the
/// `tests` module below. Returns whether anything was written.
fn emit_cursor_shape_if_changed(
    out: &mut impl Write,
    prev: Option<&'static str>,
    wanted: Option<&'static str>,
) -> std::io::Result<bool> {
    if wanted == prev {
        return Ok(false);
    }
    match wanted {
        Some(seq) => write!(out, "{}", seq)?,
        None => write!(out, "\x1b[0 q")?,
    }
    Ok(true)
}

#[cfg(test)]
mod cursor_shape_tests {
    use super::*;

    fn setup() -> Interp {
        let mut interp = elisp::new_interp();
        core::init_editor(&mut interp);
        interp
    }

    /// `Flow` (elisp's error type) isn't `Debug`, so these tests can't
    /// just `.unwrap()` an `eval_source` result — mirrors the `run()`
    /// helper every `crates/core/tests/*.rs` file defines for the same
    /// reason.
    fn run(interp: &mut Interp, src: &str) {
        if let Err(flow) = interp.eval_source(src) {
            panic!("{}: {}", src, interp.describe_flow(&flow));
        }
    }

    #[test]
    fn nil_by_default_means_no_escape() {
        let interp = setup();
        assert_eq!(cursor_type_escape(&interp), None);
    }

    /// M32: migrated from the self-made `cursor-shape` ('block/'bar) to
    /// GNU Emacs's own `cursor-type` ('box/'bar/'hbar) — see
    /// `cursor_type_escape`'s doc comment.
    #[test]
    fn box_bar_and_hbar_map_to_their_decscusr_codes() {
        let mut interp = setup();
        run(&mut interp, "(setq cursor-type 'box)");
        assert_eq!(cursor_type_escape(&interp), Some("\x1b[2 q"));
        run(&mut interp, "(setq cursor-type 'bar)");
        assert_eq!(cursor_type_escape(&interp), Some("\x1b[6 q"));
        run(&mut interp, "(setq cursor-type 'hbar)");
        assert_eq!(cursor_type_escape(&interp), Some("\x1b[4 q"));
    }

    #[test]
    fn an_unrecognized_value_is_treated_like_nil() {
        let mut interp = setup();
        run(&mut interp, "(setq cursor-type 'hollow)");
        assert_eq!(cursor_type_escape(&interp), None);
        run(&mut interp, "(setq cursor-type \"box\")");
        assert_eq!(cursor_type_escape(&interp), None);
    }

    #[test]
    fn nil_writes_nothing() {
        let mut buf = Vec::new();
        let wrote = emit_cursor_shape_if_changed(&mut buf, None, None).unwrap();
        assert!(!wrote);
        assert!(buf.is_empty());
    }

    #[test]
    fn first_non_nil_shape_is_written() {
        let mut buf = Vec::new();
        let wrote = emit_cursor_shape_if_changed(&mut buf, None, Some("\x1b[2 q")).unwrap();
        assert!(wrote);
        assert_eq!(buf, b"\x1b[2 q");
    }

    #[test]
    fn unchanged_shape_is_not_rewritten() {
        let mut buf = Vec::new();
        let wrote =
            emit_cursor_shape_if_changed(&mut buf, Some("\x1b[2 q"), Some("\x1b[2 q")).unwrap();
        assert!(!wrote);
        assert!(buf.is_empty());
    }

    /// Turning cursor-type back to nil mid-session must reset the
    /// terminal cursor, not leave the last box/bar stuck until exit
    /// (the M29 review finding this originally fixed for `cursor-shape`;
    /// evil-mode's own off-switch no longer takes this path since M32 --
    /// see `emit_cursor_shape_if_changed`'s doc comment -- but the reset
    /// itself must still work for a direct `(setq cursor-type nil)`).
    #[test]
    fn clearing_a_previously_set_shape_emits_a_reset() {
        let mut buf = Vec::new();
        let wrote = emit_cursor_shape_if_changed(&mut buf, Some("\x1b[2 q"), None).unwrap();
        assert!(wrote);
        assert_eq!(buf, b"\x1b[0 q");
    }

    #[test]
    fn changed_shape_is_rewritten() {
        let mut buf = Vec::new();
        let wrote =
            emit_cursor_shape_if_changed(&mut buf, Some("\x1b[2 q"), Some("\x1b[6 q")).unwrap();
        assert!(wrote);
        assert_eq!(buf, b"\x1b[6 q");
    }

    /// End-to-end through `draw` itself: setting `cursor-type` emits
    /// the escape exactly once across repeated draws with no further
    /// change (mirrors `prev_grid`'s "no visible change, no bytes"
    /// guarantee for cell content).
    #[test]
    fn draw_emits_the_escape_once_then_stays_quiet() {
        let mut interp = setup();
        let ed = core::editor::editor(&interp);
        ed.borrow_mut().frame = (20, 5);
        run(&mut interp, "(setq cursor-type 'bar)");

        let mut prev_grid = None;
        let mut prev_shape = None;
        let mut buf = Vec::new();
        draw(&interp, &ed, &mut buf, &mut prev_grid, &mut prev_shape).unwrap();
        let first_len = buf.len();
        assert!(
            buf.windows(5).any(|w| w == b"\x1b[6 q"),
            "expected the bar escape in the first draw: {:?}",
            buf
        );

        buf.clear();
        draw(&interp, &ed, &mut buf, &mut prev_grid, &mut prev_shape).unwrap();
        assert!(
            buf.is_empty(),
            "second draw with no change should write nothing, got {:?} (first draw was {} bytes)",
            buf,
            first_len
        );
    }
}

/// Coverage for the "crossterm raw event → `Key`" layer itself (M64).
/// `crates/core/tests/core_tests.rs`'s `undo_redo` goes through
/// `keymap::parse_kbd`, which never touches `convert_key` — this is the
/// only place that layer is exercised at all, which is why the `C-/` /
/// `C-_` mixup (see `convert_key`'s doc comment above) went unnoticed
/// for as long as it did.
#[cfg(test)]
mod convert_key_tests {
    use super::*;
    use core::keymap::parse_kbd;

    fn kbd_key(desc: &str) -> Key {
        parse_kbd(desc).unwrap()[0].clone()
    }

    fn kev(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[cfg(unix)]
    #[test]
    fn c0_keys_match_their_kbd_bindings() {
        for (c, desc) in [('4', "C-\\"), ('5', "C-]"), ('6', "C-^"), ('7', "C-_")] {
            let got = convert_key(kev(KeyCode::Char(c), KeyModifiers::CONTROL)).unwrap();
            assert_eq!(got, kbd_key(desc), "Char({c:?})+CONTROL vs {desc:?}");
        }
    }

    /// The actual bug: on a real terminal, both physical `Ctrl+/` and
    /// `Ctrl+_` arrive as the single byte 0x1F, which crossterm reports
    /// as `Char('7') + CONTROL`. That must decode to `Key::Char(31)`
    /// (`"C-_"`'s encoding), the binding simple.el's undo actually uses.
    #[cfg(unix)]
    #[test]
    fn ctrl_7_is_the_shared_undo_byte() {
        let got = convert_key(kev(KeyCode::Char('7'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(got, Key::Char(31));
    }

    #[test]
    fn plain_ctrl_a_is_unaffected() {
        let got = convert_key(kev(KeyCode::Char('a'), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(got, kbd_key("C-a"));
    }

    #[test]
    fn ctrl_space_is_unaffected() {
        let got = convert_key(kev(KeyCode::Char(' '), KeyModifiers::CONTROL)).unwrap();
        assert_eq!(got, kbd_key("C-SPC"));
    }

    #[test]
    fn unmodified_digit_keys_pass_through_as_plain_chars() {
        assert_eq!(
            convert_key(kev(KeyCode::Char('4'), KeyModifiers::NONE)).unwrap(),
            Key::Char('4' as i64)
        );
        assert_eq!(
            convert_key(kev(KeyCode::Char('7'), KeyModifiers::NONE)).unwrap(),
            Key::Char('7' as i64)
        );
    }

    #[cfg(unix)]
    #[test]
    fn meta_stacks_on_the_c0_remap() {
        let got = convert_key(kev(
            KeyCode::Char('7'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        ))
        .unwrap();
        assert_eq!(got, Key::Char(31 | META));
    }

    #[test]
    fn meta_alone_on_a_digit_does_not_trigger_the_c0_remap() {
        let got = convert_key(kev(KeyCode::Char('4'), KeyModifiers::ALT)).unwrap();
        assert_eq!(got, Key::Char('4' as i64 | META));
    }

    #[test]
    fn special_keys_unchanged() {
        assert_eq!(
            convert_key(kev(KeyCode::Enter, KeyModifiers::NONE)).unwrap(),
            Key::Char(13)
        );
        assert_eq!(
            convert_key(kev(KeyCode::Tab, KeyModifiers::NONE)).unwrap(),
            Key::Char(9)
        );
        assert_eq!(
            convert_key(kev(KeyCode::Backspace, KeyModifiers::NONE)).unwrap(),
            Key::Char(127)
        );
        assert_eq!(
            convert_key(kev(KeyCode::Esc, KeyModifiers::NONE)).unwrap(),
            Key::Char(27)
        );
        assert_eq!(
            convert_key(kev(KeyCode::Up, KeyModifiers::NONE)).unwrap(),
            Key::Sym("up".into())
        );
    }
}
