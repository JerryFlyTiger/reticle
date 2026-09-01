use crate::error::Flow;
use crate::interp::Interp;
use crate::value::Value;

pub enum ReadError {
    /// Ran out of input mid-form; a REPL should ask for more input.
    Incomplete,
    Syntax(String),
}

impl ReadError {
    pub fn into_flow(self, interp: &mut Interp) -> Flow {
        match self {
            ReadError::Incomplete => {
                let e = interp.syms.end_of_file;
                interp.signal(e, vec![])
            }
            ReadError::Syntax(msg) => {
                let e = interp.syms.invalid_read_syntax;
                interp.signal(e, vec![Value::string(msg)])
            }
        }
    }
}

pub struct Reader<'a> {
    chars: Vec<char>,
    pos: usize,
    _src: &'a str,
}

impl<'a> Reader<'a> {
    pub fn new(src: &'a str) -> Reader<'a> {
        Reader {
            chars: src.chars().collect(),
            pos: 0,
            _src: src,
        }
    }

    /// Char position just past the last form read (for read-from-string).
    pub fn pos(&self) -> usize {
        self.pos
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.pos += 1;
                }
                Some(';') => {
                    while let Some(c) = self.next() {
                        if c == '\n' {
                            break;
                        }
                    }
                }
                _ => return,
            }
        }
    }

    /// Read one form. Ok(None) = clean EOF.
    pub fn read(&mut self, interp: &mut Interp) -> Result<Option<Value>, ReadError> {
        self.skip_ws_and_comments();
        if self.peek().is_none() {
            return Ok(None);
        }
        self.read_form(interp).map(Some)
    }

    fn read_form(&mut self, interp: &mut Interp) -> Result<Value, ReadError> {
        self.skip_ws_and_comments();
        let c = self.peek().ok_or(ReadError::Incomplete)?;
        match c {
            '(' => {
                self.pos += 1;
                self.read_list(interp, ')')
            }
            '[' => {
                self.pos += 1;
                let items = self.read_vector_items(interp)?;
                Ok(Value::Vector(std::rc::Rc::new(std::cell::RefCell::new(
                    items,
                ))))
            }
            ')' | ']' => Err(ReadError::Syntax(format!("unexpected `{}`", c))),
            '\'' => {
                self.pos += 1;
                let form = self.read_form(interp)?;
                Ok(quote_with(interp.syms.quote, form))
            }
            '`' => {
                self.pos += 1;
                let form = self.read_form(interp)?;
                Ok(quote_with(interp.syms.backquote, form))
            }
            ',' => {
                self.pos += 1;
                let sym = if self.peek() == Some('@') {
                    self.pos += 1;
                    interp.syms.unquote_splicing
                } else {
                    interp.syms.unquote
                };
                let form = self.read_form(interp)?;
                Ok(quote_with(sym, form))
            }
            '"' => {
                self.pos += 1;
                self.read_string()
            }
            '?' => {
                self.pos += 1;
                self.read_char()
            }
            '#' => {
                self.pos += 1;
                match self.peek() {
                    Some('\'') => {
                        self.pos += 1;
                        let form = self.read_form(interp)?;
                        Ok(quote_with(interp.syms.function, form))
                    }
                    Some('x') | Some('X') => {
                        self.pos += 1;
                        self.read_radix_int(16)
                    }
                    Some('b') | Some('B') => {
                        self.pos += 1;
                        self.read_radix_int(2)
                    }
                    Some('o') | Some('O') => {
                        self.pos += 1;
                        self.read_radix_int(8)
                    }
                    other => Err(ReadError::Syntax(format!(
                        "unsupported #-syntax: #{:?}",
                        other
                    ))),
                }
            }
            _ => self.read_atom(interp),
        }
    }

    fn read_list(&mut self, interp: &mut Interp, close: char) -> Result<Value, ReadError> {
        let mut items: Vec<Value> = Vec::new();
        let mut tail = Value::Nil;
        loop {
            self.skip_ws_and_comments();
            match self.peek() {
                None => return Err(ReadError::Incomplete),
                Some(c) if c == close => {
                    self.pos += 1;
                    break;
                }
                Some('.') if self.is_dot_token() => {
                    if items.is_empty() {
                        return Err(ReadError::Syntax("dot at start of list".into()));
                    }
                    self.pos += 1;
                    tail = self.read_form(interp)?;
                    self.skip_ws_and_comments();
                    match self.next() {
                        Some(c) if c == close => break,
                        Some(c) => {
                            return Err(ReadError::Syntax(format!(
                                "expected `{}` after dotted tail, got `{}`",
                                close, c
                            )))
                        }
                        None => return Err(ReadError::Incomplete),
                    }
                }
                _ => items.push(self.read_form(interp)?),
            }
        }
        let mut acc = tail;
        for v in items.into_iter().rev() {
            acc = Value::cons(v, acc);
        }
        Ok(acc)
    }

    /// True if the '.' at pos is a standalone dot (not part of a number/symbol).
    fn is_dot_token(&self) -> bool {
        match self.chars.get(self.pos + 1) {
            None => true,
            Some(c) => c.is_whitespace() || *c == '(' || *c == ')' || *c == ';' || *c == '"',
        }
    }

    fn read_vector_items(&mut self, interp: &mut Interp) -> Result<Vec<Value>, ReadError> {
        let mut items = Vec::new();
        loop {
            self.skip_ws_and_comments();
            match self.peek() {
                None => return Err(ReadError::Incomplete),
                Some(']') => {
                    self.pos += 1;
                    return Ok(items);
                }
                _ => items.push(self.read_form(interp)?),
            }
        }
    }

    fn read_string(&mut self) -> Result<Value, ReadError> {
        let mut s = String::new();
        loop {
            match self.next() {
                None => return Err(ReadError::Incomplete),
                Some('"') => return Ok(Value::string(s)),
                Some('\\') => match self.next() {
                    None => return Err(ReadError::Incomplete),
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some('r') => s.push('\r'),
                    Some('e') => s.push('\u{1b}'),
                    Some('0') => s.push('\0'),
                    Some('\n') => {} // line continuation
                    Some(c) => s.push(c),
                },
                Some(c) => s.push(c),
            }
        }
    }

    /// Character literal after `?`. Supports ?a ?\n ?\t ?\\ ?\C-x ?\M-x etc.
    fn read_char(&mut self) -> Result<Value, ReadError> {
        let c = self.next().ok_or(ReadError::Incomplete)?;
        if c != '\\' {
            return Ok(Value::Int(c as i64));
        }
        let e = self.next().ok_or(ReadError::Incomplete)?;
        let code = match e {
            'n' => '\n' as i64,
            't' => '\t' as i64,
            'r' => '\r' as i64,
            'e' => 0x1b,
            's' => ' ' as i64,
            'd' => 0x7f,
            '0' => 0,
            'C' => {
                if self.next() != Some('-') {
                    return Err(ReadError::Syntax("expected `-` after ?\\C".into()));
                }
                let base = self.read_char()?;
                match base {
                    Value::Int(b) => ctrl_code(b),
                    _ => unreachable!(),
                }
            }
            'M' => {
                if self.next() != Some('-') {
                    return Err(ReadError::Syntax("expected `-` after ?\\M".into()));
                }
                let base = self.read_char()?;
                match base {
                    Value::Int(b) => b | (1 << 27),
                    _ => unreachable!(),
                }
            }
            other => other as i64,
        };
        Ok(Value::Int(code))
    }

    fn read_radix_int(&mut self, radix: u32) -> Result<Value, ReadError> {
        let mut s = String::new();
        if self.peek() == Some('-') || self.peek() == Some('+') {
            s.push(self.next().unwrap());
        }
        while let Some(c) = self.peek() {
            if c.is_digit(radix) {
                s.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        i64::from_str_radix(&s, radix)
            .map(Value::Int)
            .map_err(|_| ReadError::Syntax(format!("invalid integer literal: {}", s)))
    }

    fn read_atom(&mut self, interp: &mut Interp) -> Result<Value, ReadError> {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if c.is_whitespace() || "()[]\";'`,".contains(c) {
                break;
            }
            if c == '\\' {
                self.pos += 1;
                match self.next() {
                    Some(esc) => s.push(esc),
                    None => return Err(ReadError::Incomplete),
                }
            } else {
                s.push(c);
                self.pos += 1;
            }
        }
        if s.is_empty() {
            return Err(ReadError::Syntax("empty atom".into()));
        }
        Ok(parse_atom(interp, &s))
    }
}

fn quote_with(sym: crate::value::SymId, form: Value) -> Value {
    Value::cons(Value::Sym(sym), Value::cons(form, Value::Nil))
}

fn ctrl_code(b: i64) -> i64 {
    // C-a..C-z map to 1..26; C-? is DEL; others set the control bit like Emacs.
    match b {
        0x3f => 0x7f,
        0x40..=0x5f => b - 0x40,
        0x61..=0x7a => b - 0x60,
        _ => b | (1 << 26),
    }
}

fn parse_atom(interp: &mut Interp, s: &str) -> Value {
    if s == "nil" {
        return Value::Nil;
    }
    if let Ok(i) = s.parse::<i64>() {
        return Value::Int(i);
    }
    // Integer literal too large for i64: a bignum (only when the whole
    // token is digits with an optional sign, so symbols stay symbols).
    if s.chars()
        .enumerate()
        .all(|(i, c)| c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))
        && s.chars().any(|c| c.is_ascii_digit())
    {
        if let Ok(b) = s.parse::<num_bigint::BigInt>() {
            return Value::big(b);
        }
    }
    // Only treat as float when it looks numeric (avoid symbols like `1+`).
    let looks_float = s.contains('.') || s.contains('e') || s.contains('E');
    if looks_float
        && s.chars()
            .next()
            .map(|c| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
            .unwrap_or(false)
    {
        if let Ok(f) = s.parse::<f64>() {
            return Value::Float(f);
        }
    }
    Value::Sym(interp.intern(s))
}
