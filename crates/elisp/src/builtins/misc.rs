use std::rc::Rc;

use super::{defun, need_int, need_list, need_str, need_sym, opt};
use crate::error::Flow;
use crate::eval::apply_function;
use crate::interp::Interp;
use crate::printer::{prin1_to_string, princ_to_string};
use crate::value::{Function, Value};

pub fn register(interp: &mut Interp) {
    // Strings.
    defun(interp, "concat", 0, None, |i, a| {
        let mut out = String::new();
        for v in a.iter() {
            match v {
                Value::Nil => {}
                Value::Str(s) => out.push_str(s),
                Value::Cons(_) => {
                    for item in need_list(i, v)? {
                        let c = need_int(i, &item)?;
                        out.push(int_to_char(i, c)?);
                    }
                }
                other => return Err(i.wrong_type("sequencep", other)),
            }
        }
        Ok(Value::string(out))
    });
    defun(interp, "substring", 1, Some(3), |i, a| {
        let s = need_str(i, &a[0])?;
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len() as i64;
        let norm = |v: i64| -> i64 {
            if v < 0 {
                (len + v).max(0)
            } else {
                v.min(len)
            }
        };
        let from = match opt(a, 1) {
            Value::Nil => 0,
            v => norm(need_int(i, &v)?),
        };
        let to = match opt(a, 2) {
            Value::Nil => len,
            v => norm(need_int(i, &v)?),
        };
        if from > to {
            let e = i.syms.args_out_of_range;
            return Err(i.signal(e, vec![a[0].clone(), Value::Int(from), Value::Int(to)]));
        }
        Ok(Value::string(
            chars[from as usize..to as usize].iter().collect::<String>(),
        ))
    });
    defun(interp, "upcase", 1, Some(1), |i, a| match &a[0] {
        Value::Str(s) => Ok(Value::string(s.to_uppercase())),
        Value::Int(c) => Ok(Value::Int(
            int_to_char(i, *c)?.to_uppercase().next().unwrap_or(' ') as i64,
        )),
        v => Err(i.wrong_type("stringp", v)),
    });
    defun(interp, "downcase", 1, Some(1), |i, a| match &a[0] {
        Value::Str(s) => Ok(Value::string(s.to_lowercase())),
        Value::Int(c) => Ok(Value::Int(
            int_to_char(i, *c)?.to_lowercase().next().unwrap_or(' ') as i64,
        )),
        v => Err(i.wrong_type("stringp", v)),
    });
    defun(interp, "capitalize", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        let mut out = String::new();
        let mut at_word_start = true;
        for c in s.chars() {
            if c.is_alphanumeric() {
                if at_word_start {
                    out.extend(c.to_uppercase());
                } else {
                    out.extend(c.to_lowercase());
                }
                at_word_start = false;
            } else {
                out.push(c);
                at_word_start = true;
            }
        }
        Ok(Value::string(out))
    });
    defun(interp, "string=", 2, Some(2), |i, a| {
        let x = string_or_symbol_name(i, &a[0])?;
        let y = string_or_symbol_name(i, &a[1])?;
        Ok(Value::bool(x == y, i.syms.t))
    });
    defun(interp, "string<", 2, Some(2), |i, a| {
        let x = string_or_symbol_name(i, &a[0])?;
        let y = string_or_symbol_name(i, &a[1])?;
        Ok(Value::bool(x < y, i.syms.t))
    });
    defun(interp, "string-prefix-p", 2, Some(2), |i, a| {
        let p = need_str(i, &a[0])?;
        let s = need_str(i, &a[1])?;
        Ok(Value::bool(s.starts_with(p.as_str()), i.syms.t))
    });
    defun(interp, "string-suffix-p", 2, Some(2), |i, a| {
        let p = need_str(i, &a[0])?;
        let s = need_str(i, &a[1])?;
        Ok(Value::bool(s.ends_with(p.as_str()), i.syms.t))
    });
    defun(interp, "string-empty-p", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        Ok(Value::bool(s.is_empty(), i.syms.t))
    });
    defun(interp, "make-string", 2, Some(2), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        let code = need_int(i, &a[1])?;
        let c = int_to_char(i, code)?;
        Ok(Value::string(std::iter::repeat_n(c, n).collect::<String>()))
    });
    defun(interp, "string-to-char", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        Ok(Value::Int(s.chars().next().map(|c| c as i64).unwrap_or(0)))
    });
    defun(interp, "char-to-string", 1, Some(1), |i, a| {
        let code = need_int(i, &a[0])?;
        let c = int_to_char(i, code)?;
        Ok(Value::string(c.to_string()))
    });
    defun(interp, "split-string", 1, Some(3), |i, a| {
        let s = need_str(i, &a[0])?;
        let parts: Vec<Value> = match opt(a, 1) {
            // Default: split on whitespace, dropping empty strings.
            Value::Nil => s.split_whitespace().map(Value::string).collect(),
            sep => {
                let sep = need_str(i, &sep)?;
                let omit_empty = opt(a, 2).truthy();
                s.split(sep.as_str())
                    .filter(|p| !omit_empty || !p.is_empty())
                    .map(Value::string)
                    .collect()
            }
        };
        Ok(Value::list(parts))
    });
    defun(interp, "string-search", 2, Some(3), |i, a| {
        let needle = need_str(i, &a[0])?;
        let hay = need_str(i, &a[1])?;
        let start = match opt(a, 2) {
            Value::Nil => 0,
            v => need_int(i, &v)?.max(0) as usize,
        };
        let hay_chars: Vec<char> = hay.chars().collect();
        if start > hay_chars.len() {
            return Ok(Value::Nil);
        }
        let tail: String = hay_chars[start..].iter().collect();
        Ok(tail
            .find(needle.as_str())
            .map(|byte| Value::Int((start + tail[..byte].chars().count()) as i64))
            .unwrap_or(Value::Nil))
    });
    defun(interp, "string-trim", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        Ok(Value::string(s.trim()))
    });
    defun(interp, "string-join", 1, Some(2), |i, a| {
        let items = need_list(i, &a[0])?;
        let sep = match opt(a, 1) {
            Value::Nil => String::new(),
            v => need_str(i, &v)?.to_string(),
        };
        let mut parts = Vec::new();
        for item in &items {
            parts.push(need_str(i, item)?.to_string());
        }
        Ok(Value::string(parts.join(&sep)))
    });
    defun(interp, "format", 1, None, |i, a| {
        let fmt = need_str(i, &a[0])?;
        format_impl(i, &fmt, &a[1..]).map(Value::string)
    });
    defun(interp, "format-message", 1, None, |i, a| {
        let fmt = need_str(i, &a[0])?;
        format_impl(i, &fmt, &a[1..]).map(Value::string)
    });
    defun(interp, "message", 1, None, |i, a| {
        let fmt = need_str(i, &a[0])?;
        let text = format_impl(i, &fmt, &a[1..])?;
        i.out(&format!("{}\n", text));
        Ok(Value::string(text))
    });

    // Printing / reading.
    defun(interp, "prin1-to-string", 1, Some(1), |i, a| {
        Ok(Value::string(prin1_to_string(i, &a[0])))
    });
    defun(interp, "prin1", 1, Some(1), |i, a| {
        let s = prin1_to_string(i, &a[0]);
        i.out(&s);
        Ok(a[0].clone())
    });
    defun(interp, "princ", 1, Some(1), |i, a| {
        let s = princ_to_string(i, &a[0]);
        i.out(&s);
        Ok(a[0].clone())
    });
    defun(interp, "print", 1, Some(1), |i, a| {
        let s = prin1_to_string(i, &a[0]);
        i.out(&format!("\n{}\n", s));
        Ok(a[0].clone())
    });
    defun(interp, "terpri", 0, Some(1), |i, _| {
        i.out("\n");
        Ok(Value::Sym(i.syms.t))
    });
    defun(interp, "read", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?;
        let src = s.to_string();
        let mut r = crate::reader::Reader::new(&src);
        match r.read(i) {
            Ok(Some(v)) => Ok(v),
            Ok(None) => {
                let e = i.syms.end_of_file;
                Err(i.signal(e, vec![]))
            }
            Err(e) => Err(e.into_flow(i)),
        }
    });
    // (read-from-string STRING &optional START) -> (OBJ . ENDPOS), GNU
    // semantics with char indices: reads one form starting at START
    // (default 0), ENDPOS is just past it. Exhausted/incomplete input
    // signals end-of-file (same condition `read` uses). The END
    // argument is not supported (v1, documented).
    defun(interp, "read-from-string", 1, Some(2), |i, a| {
        let s = need_str(i, &a[0])?.to_string();
        let start = match opt(a, 1) {
            Value::Nil => 0usize,
            v => need_int(i, &v)?.max(0) as usize,
        };
        let sub: String = s.chars().skip(start).collect();
        let mut r = crate::reader::Reader::new(&sub);
        match r.read(i) {
            Ok(Some(v)) => Ok(Value::cons(v, Value::Int((start + r.pos()) as i64))),
            Ok(None) => {
                let e = i.syms.end_of_file;
                Err(i.signal(e, vec![]))
            }
            Err(e) => Err(e.into_flow(i)),
        }
    });

    // Function application.
    defun(interp, "funcall", 1, None, |i, a| {
        let f = a[0].clone();
        apply_function(i, &f, a[1..].to_vec().into())
    });
    defun(interp, "apply", 1, None, |i, a| {
        let f = a[0].clone();
        let mut args: Vec<Value> = Vec::new();
        if a.len() > 1 {
            args.extend_from_slice(&a[1..a.len() - 1]);
            args.extend(need_list(i, &a[a.len() - 1])?);
        }
        apply_function(i, &f, args.into())
    });
    defun(interp, "eval", 1, Some(2), |i, a| {
        let lexical = opt(a, 1).truthy();
        let saved = i.lexical_binding;
        i.lexical_binding = lexical;
        let r = crate::eval::eval(i, &a[0].clone(), &None);
        i.lexical_binding = saved;
        r
    });
    defun(interp, "identity", 1, Some(1), |_, a| Ok(a[0].clone()));
    defun(interp, "ignore", 0, None, |_, _| Ok(Value::Nil));
    defun(interp, "mapcar", 2, Some(2), |i, a| {
        let f = a[0].clone();
        let items = sequence_items(i, &a[1])?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            out.push(apply_function(i, &f, smallvec::smallvec![item])?);
        }
        Ok(Value::list(out))
    });
    defun(interp, "mapc", 2, Some(2), |i, a| {
        let f = a[0].clone();
        for item in sequence_items(i, &a[1])? {
            apply_function(i, &f, smallvec::smallvec![item])?;
        }
        Ok(a[1].clone())
    });
    defun(interp, "mapconcat", 2, Some(3), |i, a| {
        let f = a[0].clone();
        let sep = match opt(a, 2) {
            Value::Nil => String::new(),
            v => need_str(i, &v)?.to_string(),
        };
        let mut parts = Vec::new();
        for item in sequence_items(i, &a[1])? {
            let r = apply_function(i, &f, smallvec::smallvec![item])?;
            parts.push(match r {
                Value::Str(s) => s.to_string(),
                other => princ_to_string(i, &other),
            });
        }
        Ok(Value::string(parts.join(&sep)))
    });
    defun(interp, "sort", 2, Some(2), |i, a| {
        let mut items = need_list(i, &a[0])?;
        let pred = a[1].clone();
        // Insertion sort so the comparison predicate can signal errors.
        for j in 1..items.len() {
            let mut k = j;
            while k > 0 {
                let less = apply_function(
                    i,
                    &pred,
                    smallvec::smallvec![items[k].clone(), items[k - 1].clone()],
                )?;
                if less.truthy() {
                    items.swap(k, k - 1);
                    k -= 1;
                } else {
                    break;
                }
            }
        }
        Ok(Value::list(items))
    });

    // Errors.
    defun(interp, "error", 1, None, |i, a| {
        let fmt = need_str(i, &a[0])?;
        let text = format_impl(i, &fmt, &a[1..])?;
        Err(i.error(text))
    });
    defun(interp, "user-error", 1, None, |i, a| {
        let fmt = need_str(i, &a[0])?;
        let text = format_impl(i, &fmt, &a[1..])?;
        let e = i.syms.user_error;
        Err(i.signal(e, vec![Value::string(text)]))
    });
    defun(interp, "signal", 2, Some(2), |_, a| {
        Err(Flow::Signal {
            error_symbol: a[0].clone(),
            data: a[1].clone(),
        })
    });
    defun(interp, "throw", 2, Some(2), |_, a| {
        Err(Flow::Throw {
            tag: a[0].clone(),
            value: a[1].clone(),
        })
    });
    defun(interp, "define-error", 2, Some(3), |i, a| {
        let sym = need_sym(i, &a[0])?;
        let msg = need_str(i, &a[1])?.to_string();
        let parents: Vec<crate::value::SymId> = match opt(a, 2) {
            Value::Nil => vec![i.syms.error],
            Value::Sym(p) => vec![p],
            v => {
                let items = need_list(i, &v)?;
                let mut out = Vec::new();
                for item in &items {
                    out.push(need_sym(i, item)?);
                }
                out
            }
        };
        i.define_error(sym, &msg, &parents);
        Ok(Value::Nil)
    });

    // Loading / features.
    defun(interp, "load", 1, Some(2), |i, a| {
        let path = need_str(i, &a[0])?;
        let resolved = resolve_load_path(i, &path);
        match resolved {
            Some(p) => i.load_file(&p),
            None => {
                if opt(a, 1).truthy() {
                    Ok(Value::Nil)
                } else {
                    Err(i.error(format!("Cannot open load file: {}", path)))
                }
            }
        }
    });
    defun(interp, "provide", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        if !i.features.contains(&id) {
            i.features.push(id);
        }
        Ok(Value::Sym(id))
    });
    defun(interp, "featurep", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        Ok(Value::bool(i.features.contains(&id), i.syms.t))
    });
    defun(interp, "require", 1, Some(3), |i, a| {
        let id = need_sym(i, &a[0])?;
        if i.features.contains(&id) {
            return Ok(Value::Sym(id));
        }
        let name = i.sym_name(id).to_string();
        match resolve_load_path(i, &name) {
            Some(p) => {
                i.load_file(&p)?;
                Ok(Value::Sym(id))
            }
            None => {
                if opt(a, 2).truthy() {
                    Ok(Value::Nil)
                } else {
                    Err(i.error(format!("Cannot find library: {}", name)))
                }
            }
        }
    });
    defun(interp, "macroexpand-1", 1, Some(1), |i, a| {
        macroexpand_once(i, &a[0].clone())
    });

    // Time (seconds since the Unix epoch as a float; enough for the
    // timing/benchmark uses elisp needs, without a full time-value type).
    defun(interp, "float-time", 0, Some(1), |_, _| {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        Ok(Value::Float(secs))
    });

    // (run-with-deadline MS FUNCTION) — call FUNCTION (no args) under an
    // MS-millisecond execution budget (M15). If the budget is exhausted,
    // `elisp-timeout` is signaled from wherever execution happens to be;
    // unwind-protect cleanups run on the way out (unbudgeted — the
    // deadline clears when it fires, same as quit-flag in real Emacs).
    // Nested budgets can only shrink: an inner call cannot extend an
    // outer deadline that is already closer.
    defun(interp, "run-with-deadline", 2, Some(2), |i, a| {
        let ms = need_int(i, &a[0])?.max(0) as u64;
        let f = a[1].clone();
        let target = std::time::Instant::now() + std::time::Duration::from_millis(ms);
        let saved = i.deadline;
        i.deadline = Some(match saved {
            Some(outer) => outer.min(target),
            None => target,
        });
        let result = apply_function(i, &f, smallvec::smallvec![]);
        i.deadline = saved;
        result
    });

    // Error hierarchy (M11): user-defined error symbols whose condition
    // chains merge their parents', so condition-case handlers written
    // against a parent catch every descendant.
    defun(interp, "define-error", 2, Some(3), |i, a| {
        let sym = need_sym(i, &a[0])?;
        let msg = need_str(i, &a[1])?.to_string();
        let parents: Vec<crate::value::SymId> = match opt(a, 2) {
            Value::Nil => vec![i.syms.error],
            Value::Sym(p) => vec![p],
            v @ Value::Cons(_) => {
                let items = need_list(i, &v)?;
                let mut ids = Vec::with_capacity(items.len());
                for it in &items {
                    ids.push(need_sym(i, it)?);
                }
                ids
            }
            other => return Err(i.wrong_type("symbolp", &other)),
        };
        i.define_error(sym, &msg, &parents);
        Ok(Value::Nil)
    });

    // Self-documentation (M11): docstrings are retained on the symbol
    // plist by defun (surviving byte-/native-compilation) and read here.
    defun(interp, "documentation", 1, Some(1), |i, a| {
        let id = need_sym(i, &a[0])?;
        let prop = i.intern("function-documentation");
        match i.plist_get(id, prop) {
            v @ Value::Str(_) => Ok(v),
            _ => Ok(Value::Nil),
        }
    });

    // Regular expressions (M11), elisp dialect: \\( \\) groups, \\|
    // alternation, \\{n,m\\} counted repetition. See crate::regex.
    defun(interp, "string-match", 2, Some(3), |i, a| {
        let pat = need_str(i, &a[0])?.to_string();
        let target = need_str(i, &a[1])?.to_string();
        let start = match opt(a, 2) {
            Value::Nil => 0,
            v => need_int(i, &v)?.max(0) as usize,
        };
        match crate::regex::string_match(i, &pat, &target, start, true)? {
            Some(idx) => Ok(Value::Int(idx as i64)),
            None => Ok(Value::Nil),
        }
    });
    // Predicate variant: same result, does NOT touch match data.
    defun(interp, "string-match-p", 2, Some(3), |i, a| {
        let pat = need_str(i, &a[0])?.to_string();
        let target = need_str(i, &a[1])?.to_string();
        let start = match opt(a, 2) {
            Value::Nil => 0,
            v => need_int(i, &v)?.max(0) as usize,
        };
        match crate::regex::string_match(i, &pat, &target, start, false)? {
            Some(idx) => Ok(Value::Int(idx as i64)),
            None => Ok(Value::Nil),
        }
    });
    defun(interp, "match-beginning", 1, Some(1), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        Ok(match i.match_data.caps.get(n) {
            Some(Some((s, _))) => Value::Int(*s as i64 + i.match_data.offset),
            _ => Value::Nil,
        })
    });
    defun(interp, "match-end", 1, Some(1), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        Ok(match i.match_data.caps.get(n) {
            Some(Some((_, e))) => Value::Int(*e as i64 + i.match_data.offset),
            _ => Value::Nil,
        })
    });
    defun(interp, "match-string", 1, Some(2), |i, a| {
        let n = need_int(i, &a[0])?.max(0) as usize;
        let Some(Some((s, e))) = i.match_data.caps.get(n).copied() else {
            return Ok(Value::Nil);
        };
        // Explicit STRING argument wins; otherwise the stored snapshot
        // of whatever was searched (string or buffer text — `target` is
        // `Rc<str>` now, M43 period 2, so the no-STRING-argument path
        // slices it by the engine's native BYTE positions
        // (`caps_bytes`, kept alongside `caps` for exactly this) — an
        // O(1) `&str` slice instead of the char-by-char `Vec<char>`
        // collect this replaces.
        let extracted: String = match opt(a, 1) {
            Value::Nil => match (
                &i.match_data.target,
                i.match_data.caps_bytes.get(n).copied(),
            ) {
                (Some(t), Some(Some((bs, be)))) => t.get(bs..be).unwrap_or_default().to_string(),
                _ => return Ok(Value::Nil),
            },
            v => need_str(i, &v)?.chars().skip(s).take(e - s).collect(),
        };
        Ok(Value::string(extracted))
    });
    defun(interp, "replace-regexp-in-string", 3, Some(3), |i, a| {
        let pat = need_str(i, &a[0])?.to_string();
        let rep = need_str(i, &a[1])?.to_string();
        let target = need_str(i, &a[2])?.to_string();
        Ok(Value::string(crate::regex::replace_all(
            i, &pat, &rep, &target,
        )?))
    });
    defun(interp, "regexp-quote", 1, Some(1), |i, a| {
        let s = need_str(i, &a[0])?.to_string();
        Ok(Value::string(crate::regex::quote(&s)))
    });
    defun(interp, "split-string", 1, Some(3), |i, a| {
        let s = need_str(i, &a[0])?.to_string();
        let sep = match opt(a, 1) {
            Value::Nil => "[ \u{c}\t\n\r\u{b}]+".to_string(),
            v => need_str(i, &v)?.to_string(),
        };
        let omit_nulls = match opt(a, 2) {
            // GNU default: omit nulls when SEPARATORS was nil.
            Value::Nil => matches!(opt(a, 1), Value::Nil),
            v => v.truthy(),
        };
        let re = crate::regex::compile(i, &sep)?;
        // M43 period 2: `Regex::search` now takes `&str` + byte
        // positions directly — `s` needs no `Vec<char>` detour anymore.
        let mut parts: Vec<Value> = Vec::new();
        let mut pos = 0usize;
        // M81 R9: ONE `Scratch` shared across this WHOLE loop, via
        // `search_with` — NOT the public `search` (which would allocate
        // a fresh `Scratch`, and so a fresh budget, on every iteration).
        // This is the third place (after `replace_all` and
        // `re-search-backward`, R5) that retries `search` in an outer
        // loop against the same string; it has the exact same
        // "individually cheap, cumulatively unbounded" gap R5 fixed —
        // reviewer's own R9 finding.
        let mut scratch = crate::regex::Scratch::new(s.len());
        while pos <= s.len() {
            let found = re
                .search_with(&s, pos, &mut scratch)
                .map_err(|_| i.regexp_too_complex())?;
            let Some(caps) = found else {
                break;
            };
            let (ms, me) = caps[0].unwrap();
            if me == ms {
                break; // zero-width separator: stop rather than loop
            }
            let piece = s[pos..ms].to_string();
            if !(omit_nulls && piece.is_empty()) {
                parts.push(Value::string(piece));
            }
            pos = me;
        }
        let tail = s[pos.min(s.len())..].to_string();
        if !(omit_nulls && tail.is_empty()) {
            parts.push(Value::string(tail));
        }
        Ok(Value::list(parts))
    });

    // Cycle collection (M9). Returns (FREED REMAINING) — cyclic objects
    // reclaimed and registry entries still live — or nil if the
    // evaluator wasn't at a quiescent point... which can't actually
    // happen from elisp: by the time this builtin runs, depth > 0!
    // So collect() gets a depth-adjusted view: the builtin itself is
    // the only frame, making this the quiescent point for elisp code.
    defun(interp, "garbage-collect", 0, Some(1), |i, a| {
        let dry_run = !opt(a, 0).is_nil();
        // The eval frames leading to this builtin all hold no cyclic
        // roots beyond what the symbol table reaches... that is NOT
        // guaranteed (caller locals!), so rather than lie about depth,
        // we conservatively treat only registry objects unreachable
        // from symbols+providers+**current live env chain** as garbage.
        // Since we can't see the Rust stack, we only run when this call
        // is the sole eval in flight (depth == 1, i.e. a top-level
        // (garbage-collect) form); deeper calls return nil and the
        // collection happens at the next natural quiescent point.
        if i.depth != 1 {
            return Ok(Value::Nil);
        }
        i.depth = 0;
        let out = crate::gc::collect(i, dry_run);
        i.depth = 1;
        match out {
            Some(o) => Ok(Value::list(vec![
                Value::Int(o.freed as i64),
                Value::Int(o.remaining as i64),
            ])),
            None => Ok(Value::Nil),
        }
    });
    defun(interp, "gc-registered-count", 0, Some(0), |i, _| {
        Ok(Value::Int(i.gc.live_count() as i64))
    });

    // Bytecode compilation (M7 performance layer).
    defun(interp, "byte-compile", 1, Some(1), |i, a| {
        byte_compile(i, &a[0])
    });
    defun(interp, "byte-code-function-p", 1, Some(1), |i, a| {
        let is = matches!(&a[0], Value::Func(f) if matches!(f.as_ref(), Function::Compiled(_)));
        Ok(Value::bool(is, i.syms.t))
    });

    // Native compilation (M7 layer-2 JIT via Cranelift). Opt-in, like
    // byte-compile: only pure-integer functions qualify (see jit.rs);
    // anything else is left byte-compiled with no error.
    defun(interp, "native-compile", 1, Some(1), |i, a| {
        native_compile(i, &a[0])
    });
    defun(interp, "native-compiled-function-p", 1, Some(1), |i, a| {
        let is = matches!(&a[0], Value::Func(f) if matches!(f.as_ref(), Function::Native(_)));
        Ok(Value::bool(is, i.syms.t))
    });
}

/// `(native-compile SYMBOL-OR-FUNCTION)`: byte-compiles first if needed,
/// then attempts native compilation. Returns the symbol/function
/// unchanged (still just byte-compiled) if the function falls outside
/// the native-eligible subset — this never errors, since "not eligible"
/// is an expected, common outcome, not a failure.
fn native_compile(interp: &mut Interp, arg: &Value) -> Result<Value, Flow> {
    let byte_compiled = byte_compile(interp, arg)?;
    let (target, sym) = if let Some(id) = interp.as_sym(&byte_compiled) {
        let f = interp.symbols[id as usize]
            .function
            .clone()
            .ok_or_else(|| {
                let e = interp.syms.void_function;
                interp.signal(e, vec![Value::Sym(id)])
            })?;
        (f, Some(id))
    } else {
        (byte_compiled.clone(), None)
    };
    let Value::Func(f) = &target else {
        return Ok(byte_compiled);
    };
    let Function::Compiled(c) = f.as_ref() else {
        return Ok(byte_compiled);
    };
    let Some(native) = crate::jit::try_compile(interp, c) else {
        return Ok(byte_compiled);
    };
    let result = Value::Func(Rc::new(Function::Native(native)));
    if let Some(id) = sym {
        interp.symbols[id as usize].function = Some(result);
        Ok(Value::Sym(id))
    } else {
        Ok(result)
    }
}

/// `(byte-compile SYMBOL-OR-FUNCTION)`: compiles a plain (non-macro)
/// interpreted function to bytecode. Given a symbol, replaces its
/// function cell in place (like real Emacs) and returns the symbol;
/// given a function value, returns the compiled function without
/// touching any symbol. Already-compiled input is returned unchanged.
fn byte_compile(interp: &mut Interp, arg: &Value) -> Result<Value, Flow> {
    if let Some(id) = interp.as_sym(arg) {
        let func = interp.symbols[id as usize]
            .function
            .clone()
            .ok_or_else(|| {
                let e = interp.syms.void_function;
                interp.signal(e, vec![Value::Sym(id)])
            })?;
        let compiled = compile_function_value(interp, &func)?;
        if let Value::Func(f) = &compiled {
            if let Function::Compiled(c) = f.as_ref() {
                *c.name.borrow_mut() = Some(id);
            }
        }
        interp.symbols[id as usize].function = Some(compiled);
        return Ok(Value::Sym(id));
    }
    compile_function_value(interp, arg)
}

fn compile_function_value(interp: &mut Interp, func: &Value) -> Result<Value, Flow> {
    let Value::Func(f) = func else {
        return Err(interp.wrong_type("functionp", func));
    };
    match f.as_ref() {
        Function::Compiled(_) => Ok(func.clone()),
        Function::Native(_) => Ok(func.clone()),
        Function::Builtin { .. } => Ok(func.clone()),
        Function::Module(_) => Ok(func.clone()),
        Function::Lambda(l) => {
            if l.is_macro {
                return Err(interp.error("byte-compile: cannot compile a macro"));
            }
            let compiled = crate::compiler::compile_parsed(
                interp,
                l.params.clone(),
                l.body.clone(),
                l.interactive.borrow().clone(),
                l.lexical,
                l.env.clone(),
            )?;
            *compiled.name.borrow_mut() = *l.name.borrow();
            Ok(Value::Func(Rc::new(Function::Compiled(compiled))))
        }
    }
}

fn int_to_char(interp: &mut Interp, c: i64) -> Result<char, Flow> {
    u32::try_from(c)
        .ok()
        .and_then(char::from_u32)
        .ok_or_else(|| interp.wrong_type("characterp", &Value::Int(c)))
}

fn string_or_symbol_name(interp: &mut Interp, v: &Value) -> Result<String, Flow> {
    match v {
        Value::Str(s) => Ok(s.to_string()),
        Value::Sym(id) => Ok(interp.sym_name(*id).to_string()),
        Value::Nil => Ok("nil".to_string()),
        _ => Err(interp.wrong_type("stringp", v)),
    }
}

fn sequence_items(interp: &mut Interp, v: &Value) -> Result<Vec<Value>, Flow> {
    match v {
        Value::Nil | Value::Cons(_) => need_list(interp, v),
        Value::Vector(items) => Ok(items.borrow().clone()),
        Value::Str(s) => Ok(s.chars().map(|c| Value::Int(c as i64)).collect()),
        _ => Err(interp.wrong_type("sequencep", v)),
    }
}

/// %s %S %d %c %x %X %o %f %% — no padding flags yet.
fn format_impl(interp: &mut Interp, fmt: &str, args: &[Value]) -> Result<String, Flow> {
    let mut out = String::new();
    let mut chars = fmt.chars().peekable();
    let mut idx = 0usize;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let spec = chars
            .next()
            .ok_or_else(|| interp.error("format string ends in %"))?;
        if spec == '%' {
            out.push('%');
            continue;
        }
        let arg = args
            .get(idx)
            .cloned()
            .ok_or_else(|| interp.error("not enough arguments for format string"))?;
        idx += 1;
        match spec {
            's' => out.push_str(&princ_to_string(interp, &arg)),
            'S' => out.push_str(&prin1_to_string(interp, &arg)),
            'd' => match &arg {
                Value::Int(n) => out.push_str(&n.to_string()),
                Value::Float(f) => out.push_str(&(*f as i64).to_string()),
                v => return Err(interp.wrong_type("numberp", v)),
            },
            'x' => out.push_str(&format!("{:x}", need_int(interp, &arg)?)),
            'X' => out.push_str(&format!("{:X}", need_int(interp, &arg)?)),
            'o' => out.push_str(&format!("{:o}", need_int(interp, &arg)?)),
            'c' => {
                let code = need_int(interp, &arg)?;
                out.push(int_to_char(interp, code)?);
            }
            'f' => match &arg {
                Value::Int(n) => out.push_str(&format!("{:.6}", *n as f64)),
                Value::Float(f) => out.push_str(&format!("{:.6}", f)),
                v => return Err(interp.wrong_type("numberp", v)),
            },
            other => return Err(interp.error(format!("invalid format operation %{}", other))),
        }
    }
    Ok(out)
}

fn resolve_load_path(interp: &Interp, name: &str) -> Option<String> {
    let candidates = |base: &str| -> Vec<String> {
        if base.ends_with(".el") {
            vec![base.to_string()]
        } else {
            vec![format!("{}.el", base), base.to_string()]
        }
    };
    if name.contains('/') {
        for c in candidates(name) {
            if std::path::Path::new(&c).is_file() {
                return Some(c);
            }
        }
        return None;
    }
    for dir in &interp.load_path {
        for c in candidates(name) {
            let p = format!("{}/{}", dir, c);
            if std::path::Path::new(&p).is_file() {
                return Some(p);
            }
        }
    }
    // Fall back to the literal name relative to cwd.
    candidates(name)
        .into_iter()
        .find(|c| std::path::Path::new(c).is_file())
}

fn macroexpand_once(interp: &mut Interp, form: &Value) -> Result<Value, Flow> {
    let head = form.car();
    if let (Value::Cons(_), Some(id)) = (form, interp.as_sym(&head)) {
        if let Ok(Value::Func(f)) = crate::eval::resolve_function_of_symbol(interp, id) {
            if let crate::value::Function::Lambda(l) = f.as_ref() {
                if l.is_macro {
                    let args = form
                        .cdr()
                        .list_to_vec()
                        .ok_or_else(|| interp.error("malformed macro call"))?;
                    return apply_function(interp, &Value::Func(f.clone()), args.into());
                }
            }
        }
    }
    Ok(form.clone())
}
