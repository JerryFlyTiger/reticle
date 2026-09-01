use std::cell::RefCell;
use std::rc::Rc;

use std::any::Any;

use elisp::value::ExtRef;
use elisp::Value;

use crate::commands::Key;

pub const KEYMAP_TAG: &str = "keymap";

pub const CTRL: i64 = 1 << 26;
pub const META: i64 = 1 << 27;

#[derive(Default)]
pub struct Keymap {
    pub entries: RefCell<Vec<(Key, Value)>>,
}

/// GC tracer (M52): walk every binding's `Value` — this is what lets
/// closures captured by keys, and nested prefix keymaps (themselves
/// stored as ordinary `Value::Ext` entries), stay reachable through the
/// GC mark phase instead of only through root providers.
fn trace_keymap(obj: &Rc<dyn Any>, sink: &mut dyn FnMut(&Value)) -> bool {
    let Some(map) = obj.clone().downcast::<Keymap>().ok() else {
        return true;
    };
    let Ok(entries) = map.entries.try_borrow() else {
        return false;
    };
    for (_, v) in entries.iter() {
        sink(v);
    }
    true
}

pub fn make_keymap() -> Value {
    Value::Ext(ExtRef::new(
        KEYMAP_TAG,
        Keymap::default(),
        Some(trace_keymap),
    ))
}

pub fn as_keymap(v: &Value) -> Option<Rc<Keymap>> {
    v.as_ext::<Keymap>(KEYMAP_TAG)
}

impl Keymap {
    pub fn get(&self, key: &Key) -> Option<Value> {
        self.entries
            .borrow()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    pub fn set(&self, key: Key, def: Value) {
        let mut entries = self.entries.borrow_mut();
        if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
            slot.1 = def;
        } else {
            entries.push((key, def));
        }
    }

    /// Bind a key sequence, creating intermediate prefix keymaps as needed.
    pub fn define_sequence(map: &Rc<Keymap>, keys: &[Key], def: Value) {
        if keys.is_empty() {
            return;
        }
        if keys.len() == 1 {
            map.set(keys[0].clone(), def);
            return;
        }
        let next = match map.get(&keys[0]).and_then(|v| as_keymap(&v)) {
            Some(sub) => sub,
            None => {
                let sub_val = make_keymap();
                let sub = as_keymap(&sub_val).unwrap();
                map.set(keys[0].clone(), sub_val);
                sub
            }
        };
        Keymap::define_sequence(&next, &keys[1..], def);
    }
}

/// Encode a control character the way the elisp reader does:
/// C-a..C-z are 1..26, otherwise set the control bit.
pub fn ctrl_encode(c: char) -> i64 {
    let b = c as i64;
    match b {
        0x3f => 0x7f,
        0x40..=0x5f => b - 0x40,
        0x61..=0x7a => b - 0x60,
        _ => b | CTRL,
    }
}

/// Parse an Emacs key description like "C-x C-f", "M-x", "<up>", "C-c t".
pub fn parse_kbd(desc: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    for word in desc.split_whitespace() {
        keys.push(parse_one(word)?);
    }
    Ok(keys)
}

fn parse_one(word: &str) -> Result<Key, String> {
    if word.starts_with('<') && word.ends_with('>') && word.len() > 2 {
        let name = &word[1..word.len() - 1];
        // `<escape>` names the same bare-ESC event as "ESC" (M28: evil.el
        // and friends bind whichever spelling is at hand); every other
        // bracketed name is a genuine function-key symbol.
        return Ok(if name == "escape" {
            Key::Char(27)
        } else {
            Key::Sym(name.to_string())
        });
    }
    let mut rest = word;
    let mut ctrl = false;
    let mut meta = false;
    loop {
        if let Some(r) = rest.strip_prefix("C-") {
            if r.is_empty() {
                return Err(format!("dangling C- in {:?}", word));
            }
            ctrl = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("M-") {
            if r.is_empty() {
                return Err(format!("dangling M- in {:?}", word));
            }
            meta = true;
            rest = r;
        } else {
            break;
        }
    }
    let base: i64 = match rest {
        "RET" => 13,
        "TAB" => 9,
        "SPC" => 32,
        "ESC" => 27,
        "DEL" => 127,
        _ => {
            let mut chars = rest.chars();
            let c = chars
                .next()
                .ok_or_else(|| format!("empty key in {:?}", word))?;
            if chars.next().is_some() {
                // Multi-char name without <>: treat as symbol (e.g. "up").
                let mut k = Key::Sym(rest.to_string());
                if ctrl || meta {
                    k = match k {
                        Key::Sym(s) => Key::Sym(format!(
                            "{}{}{}",
                            if ctrl { "C-" } else { "" },
                            if meta { "M-" } else { "" },
                            s
                        )),
                        other => other,
                    };
                }
                return Ok(k);
            }
            c as i64
        }
    };
    let mut code = base;
    if ctrl {
        code = match char::from_u32(base as u32) {
            Some(c) => ctrl_encode(c),
            None => base | CTRL,
        };
    }
    if meta {
        code |= META;
    }
    Ok(Key::Char(code))
}

/// Human-readable description of a key (for echoing prefixes and errors).
pub fn key_description(key: &Key) -> String {
    match key {
        Key::Sym(s) => format!("<{}>", s),
        Key::Char(code) => {
            let mut out = String::new();
            let mut c = *code;
            if c & META != 0 {
                out.push_str("M-");
                c &= !META;
            }
            if c & CTRL != 0 {
                out.push_str("C-");
                c &= !CTRL;
            }
            match c {
                13 => out.push_str("RET"),
                9 => out.push_str("TAB"),
                32 => out.push_str("SPC"),
                27 => out.push_str("ESC"),
                127 => out.push_str("DEL"),
                1..=26 => {
                    out.push_str("C-");
                    out.push(char::from_u32((c + 96) as u32).unwrap());
                }
                // `ctrl_encode` (like `parse_one`/`parse_kbd`) leaves
                // C-@ and C-\ / C-] / C-^ / C-_ as their raw ASCII C0
                // values (0, 28..=31) rather than setting the CTRL bit.
                // Without these arms they fell through to the
                // printable-char branch below and pushed the raw
                // unprintable C0 byte straight into the echo area.
                // 28..=31 is what the M64 `convert_key` C0 remap
                // (`crates/frontend-tui/src/lib.rs`) hands the keymap for
                // every terminal `C-\`/`C-]`/`C-^`/`C-_`/`C-/` press, so
                // those four are on the hot path for the "... is
                // undefined" echo. 0 (C-@) is NOT produced by that remap
                // — it only arrives via `ctrl_encode`'s ordinary path
                // (`parse_kbd("C-@")`, or a CSI-u terminal reporting an
                // already-disambiguated `Char('@') + CONTROL`) — but it
                // has the same unprintable-echo problem, so it gets an
                // arm too.
                0 => out.push_str("C-@"),
                28 => out.push_str("C-\\"),
                29 => out.push_str("C-]"),
                30 => out.push_str("C-^"),
                31 => out.push_str("C-_"),
                _ => match char::from_u32(c as u32) {
                    Some(ch) => out.push(ch),
                    None => out.push_str(&format!("#{}", c)),
                },
            }
            out
        }
    }
}

pub fn key_sequence_description(keys: &[Key]) -> String {
    keys.iter()
        .map(key_description)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Recursively flatten a keymap's bindings into full key sequences (M67,
/// `all-key-bindings`): a nested prefix keymap under `C-h` (e.g. the
/// existing `C-h .`) produces an entry keyed by the WHOLE sequence
/// `[C-h, .]`, not just `[C-h]`. Depth-capped at 9 (`depth` starts at 0
/// and the guard below is `depth > 8`, so depths 0..=8 all recurse) — a
/// keymap that (however unusually) contains itself as a sub-keymap would
/// otherwise recurse forever; past the cap that branch is silently
/// dropped, not an error, since this is an enumeration helper, not a
/// correctness-critical lookup.
pub fn enumerate_bindings(map: &Rc<Keymap>, depth: usize) -> Vec<(Vec<Key>, Value)> {
    let mut out = Vec::new();
    if depth > 8 {
        return out;
    }
    for (key, val) in map.entries.borrow().iter() {
        if let Some(sub) = as_keymap(val) {
            for (mut seq, v) in enumerate_bindings(&sub, depth + 1) {
                seq.insert(0, key.clone());
                out.push((seq, v));
            }
        } else {
            out.push((vec![key.clone()], val.clone()));
        }
    }
    out
}

#[cfg(test)]
mod key_description_tests {
    use super::*;

    #[test]
    fn c0_control_codes_get_readable_names() {
        assert_eq!(key_description(&Key::Char(0)), "C-@");
        assert_eq!(key_description(&Key::Char(28)), "C-\\");
        assert_eq!(key_description(&Key::Char(29)), "C-]");
        assert_eq!(key_description(&Key::Char(30)), "C-^");
        assert_eq!(key_description(&Key::Char(31)), "C-_");
    }

    /// `parse_kbd` and `key_description` should round-trip for every C0
    /// code that a real terminal can send raw (this is what M64's
    /// `convert_key` C0 remap relies on producing readable messages
    /// for). `C-@` rides along because it shares the encoding shape
    /// (`ctrl_encode` leaves it as raw 0), even though the remap itself
    /// never produces it — see `key_description`'s comment.
    #[test]
    fn c0_codes_round_trip_through_parse_kbd() {
        for desc in ["C-@", "C-\\", "C-]", "C-^", "C-_"] {
            let keys = parse_kbd(desc).unwrap();
            assert_eq!(keys.len(), 1);
            assert_eq!(key_description(&keys[0]), desc, "round-trip of {desc:?}");
        }
    }

    #[test]
    fn existing_descriptions_are_unaffected() {
        assert_eq!(key_description(&parse_kbd("C-a").unwrap()[0]), "C-a");
        assert_eq!(key_description(&parse_kbd("RET").unwrap()[0]), "RET");
        assert_eq!(key_description(&parse_kbd("TAB").unwrap()[0]), "TAB");
        assert_eq!(key_description(&parse_kbd("SPC").unwrap()[0]), "SPC");
        assert_eq!(key_description(&parse_kbd("ESC").unwrap()[0]), "ESC");
        assert_eq!(key_description(&parse_kbd("DEL").unwrap()[0]), "DEL");
        assert_eq!(key_description(&parse_kbd("M-C-_").unwrap()[0]), "M-C-_");
    }

    /// M79: `simple.el` binds `M-!`/`M-|`/`M-&` globally to
    /// `shell-command`/`shell-command-on-region`/`async-shell-command`
    /// -- confirm `parse_kbd` accepts all three as a single key (not,
    /// say, misparsing `M-|` as two tokens) and that `key_description`
    /// round-trips each one back to the same spelling.
    #[test]
    fn m79_shell_command_keys_round_trip_through_parse_kbd() {
        for desc in ["M-!", "M-|", "M-&"] {
            let keys = parse_kbd(desc).unwrap();
            assert_eq!(keys.len(), 1, "expected a single key for {desc:?}");
            assert_eq!(key_description(&keys[0]), desc, "round-trip of {desc:?}");
        }
    }
}
