//! M20: elisp-syntax sexp scanning for forward-sexp/backward-sexp.
//! Hand-rolled over a char slice — no syntax tables (v1, documented):
//! understands `()`/`[]`, strings with `\` escapes, `;` line comments,
//! and `?X` / `?\X` char literals at token start.

/// Is `c` part of an atom (symbol/number) body?
fn atom_char(c: char) -> bool {
    !c.is_whitespace() && !matches!(c, '(' | ')' | '[' | ']' | '"' | ';')
}

fn skip_ws_and_comments(chars: &[char], mut pos: usize) -> usize {
    while pos < chars.len() {
        let c = chars[pos];
        if c.is_whitespace() {
            pos += 1;
        } else if c == ';' {
            while pos < chars.len() && chars[pos] != '\n' {
                pos += 1;
            }
        } else {
            break;
        }
    }
    pos
}

/// End position of the char literal starting at `pos` (chars[pos]=='?').
fn char_literal_end(chars: &[char], pos: usize) -> usize {
    let mut p = pos + 1;
    if p < chars.len() && chars[p] == '\\' {
        p += 1;
    }
    if p < chars.len() {
        p += 1;
    }
    p
}

/// Scan one sexp forward from `pos`; returns the end position (just
/// past the sexp).
pub fn forward_one(chars: &[char], pos: usize) -> Result<usize, String> {
    let mut pos = skip_ws_and_comments(chars, pos);
    // Reader prefixes: quote/backquote/unquote(-splicing)/function.
    while pos < chars.len() && matches!(chars[pos], '\'' | '`' | ',' | '#' | '@') {
        pos += 1;
    }
    if pos >= chars.len() {
        return Err("End of buffer during sexp scan".to_string());
    }
    match chars[pos] {
        '(' | '[' => {
            let mut depth = 0usize;
            let mut p = pos;
            while p < chars.len() {
                let c = chars[p];
                match c {
                    '"' => p = string_end(chars, p)?,
                    ';' => {
                        while p < chars.len() && chars[p] != '\n' {
                            p += 1;
                        }
                    }
                    '?' if is_token_start(chars, p) => p = char_literal_end(chars, p),
                    '(' | '[' => {
                        depth += 1;
                        p += 1;
                    }
                    ')' | ']' => {
                        depth -= 1;
                        p += 1;
                        if depth == 0 {
                            return Ok(p);
                        }
                    }
                    _ => p += 1,
                }
            }
            Err("Unbalanced parentheses".to_string())
        }
        '"' => string_end(chars, pos),
        ')' | ']' => Err("Unbalanced parentheses".to_string()),
        '?' => Ok(char_literal_end(chars, pos)),
        _ => {
            let mut p = pos;
            while p < chars.len() && atom_char(chars[p]) {
                p += 1;
            }
            Ok(p)
        }
    }
}

/// End position of the string starting at `pos` (chars[pos]=='"').
fn string_end(chars: &[char], pos: usize) -> Result<usize, String> {
    let mut p = pos + 1;
    while p < chars.len() {
        match chars[p] {
            '\\' => p += 2,
            '"' => return Ok(p + 1),
            _ => p += 1,
        }
    }
    Err("Unterminated string".to_string())
}

/// A '?' at `pos` opens a char literal only at a token boundary
/// (`foo?` stays one symbol).
fn is_token_start(chars: &[char], pos: usize) -> bool {
    pos == 0 || !atom_char(chars[pos - 1]) || matches!(chars[pos - 1], '(' | '[')
}

/// Start position of the sexp ending nearest before `target`: forward
/// event scan from the buffer start, keeping the complete sexp (atom,
/// string, or balanced group) with the greatest end <= target —
/// choosing the greatest end naturally prefers the enclosing group over
/// its last child (the group's `)` ends later).
pub fn backward_one(chars: &[char], target: usize) -> Result<usize, String> {
    let mut best: Option<(usize, usize)> = None; // (start, end)
    let mut stack: Vec<usize> = Vec::new();
    let mut p = 0usize;
    while p < chars.len() && p < target {
        let c = chars[p];
        if c.is_whitespace() {
            p += 1;
            continue;
        }
        match c {
            ';' => {
                while p < chars.len() && chars[p] != '\n' {
                    p += 1;
                }
            }
            '"' => {
                let s = p;
                let e = string_end(chars, p).unwrap_or(chars.len());
                if e <= target {
                    consider(&mut best, s, e);
                }
                p = e;
            }
            '?' if is_token_start(chars, p) => {
                let s = p;
                let e = char_literal_end(chars, p);
                if e <= target {
                    consider(&mut best, s, e);
                }
                p = e;
            }
            '(' | '[' => {
                stack.push(p);
                p += 1;
            }
            ')' | ']' => {
                p += 1;
                if let Some(s) = stack.pop() {
                    if p <= target {
                        consider(&mut best, s, p);
                    }
                }
            }
            '\'' | '`' | ',' | '#' | '@' => p += 1,
            _ => {
                let s = p;
                while p < chars.len() && atom_char(chars[p]) {
                    p += 1;
                }
                if p <= target {
                    consider(&mut best, s, p);
                }
            }
        }
    }
    match best {
        Some((s, _)) => Ok(s),
        None => Err("No sexp before point".to_string()),
    }
}

fn consider(best: &mut Option<(usize, usize)>, s: usize, e: usize) {
    if best.map(|(_, be)| e >= be).unwrap_or(true) {
        *best = Some((s, e));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cv(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn forward_basics() {
        assert_eq!(forward_one(&cv("(a b)"), 0), Ok(5));
        assert_eq!(forward_one(&cv("  foo bar"), 0), Ok(5));
        assert_eq!(forward_one(&cv("\"x)y\" z"), 0), Ok(5));
        assert_eq!(forward_one(&cv("(a \"x)\" ;)\n b)"), 0), Ok(14));
        assert_eq!(forward_one(&cv("'(a b)"), 0), Ok(6));
        assert_eq!(forward_one(&cv("(a ?\\) b)"), 0), Ok(9)); // ?\) char literal
        assert!(forward_one(&cv("(a b"), 0).is_err());
        assert!(forward_one(&cv(")"), 0).is_err());
    }

    #[test]
    fn backward_basics() {
        // "(a b) " with target after the group → start 0.
        assert_eq!(backward_one(&cv("(a b) "), 6), Ok(0));
        // Inside "(a b |)" → start of b.
        assert_eq!(backward_one(&cv("(a b )"), 5), Ok(3));
        // "((x) )" from before the close → start of (x).
        assert_eq!(backward_one(&cv("((x) )"), 5), Ok(1));
        // After a string.
        assert_eq!(backward_one(&cv("x \"a b\" "), 8), Ok(2));
        assert!(backward_one(&cv("   "), 3).is_err());
    }
}
