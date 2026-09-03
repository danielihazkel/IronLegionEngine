//! A small JSON5 parser that keeps positions (T1-020, REQ-TECH-005).
//!
//! Supported: comments (`//`, `/* */`), trailing commas, unquoted keys,
//! single- and double-quoted strings with every JSON5 escape and line
//! continuation, hex integers, leading/trailing decimal points, explicit `+`,
//! Unicode whitespace and line terminators. Not supported, by design:
//! `Infinity` and `NaN` (content never wants them and `serde_json` cannot
//! hold them). Duplicate keys in one object are an error.

use super::value::{FileId, Key, Num, Span, SpannedValue, ValueKind};

/// A syntax error at a position; parsing stops at the first one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub span: Span,
    pub message: String,
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}: {}", self.span.line, self.span.col, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parses one JSON5 document (any value at the top level).
pub fn parse_json5(src: &str, file: FileId) -> Result<SpannedValue, ParseError> {
    let mut p = Parser {
        chars: src.chars().collect(),
        pos: 0,
        line: 1,
        col: 1,
        file,
    };
    p.skip_trivia()?;
    let value = p.value()?;
    p.skip_trivia()?;
    if let Some(c) = p.peek() {
        return Err(p.error(format!("unexpected {} after the document", describe(c))));
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    line: u32,
    col: u32,
    file: FileId,
}

fn describe(c: char) -> String {
    match c {
        '\n' | '\r' | '\u{2028}' | '\u{2029}' => "end of line".to_string(),
        c if c.is_whitespace() => "whitespace".to_string(),
        c => format!("{c:?}"),
    }
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphabetic()
}

fn is_ident_part(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit() || matches!(c, '\u{200C}' | '\u{200D}')
}

impl Parser {
    fn span(&self) -> Span {
        Span {
            file: self.file,
            line: self.line,
            col: self.col,
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            span: self.span(),
            message: message.into(),
        }
    }

    fn error_at(&self, span: Span, message: impl Into<String>) -> ParseError {
        ParseError {
            span,
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    /// Consumes one character, tracking lines (CRLF counts once).
    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += 1;
        if c == '\r' && self.peek() == Some('\n') {
            // The LF finishes this line; handled on its own bump.
            self.col += 1;
        } else if is_line_terminator(c) {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char, what: &str) -> Result<(), ParseError> {
        match self.peek() {
            Some(got) if got == c => {
                self.bump();
                Ok(())
            }
            Some(got) => Err(self.error(format!("expected {what}, found {}", describe(got)))),
            None => Err(self.error(format!("expected {what}, found end of file"))),
        }
    }

    fn skip_trivia(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() || c == '\u{FEFF}' => {
                    self.bump();
                }
                Some('/') => match self.peek_at(1) {
                    Some('/') => {
                        while let Some(c) = self.peek() {
                            if is_line_terminator(c) {
                                break;
                            }
                            self.bump();
                        }
                    }
                    Some('*') => {
                        let start = self.span();
                        self.bump();
                        self.bump();
                        loop {
                            match self.peek() {
                                None => {
                                    return Err(self.error_at(start, "unterminated block comment"));
                                }
                                Some('*') if self.peek_at(1) == Some('/') => {
                                    self.bump();
                                    self.bump();
                                    break;
                                }
                                Some(_) => {
                                    self.bump();
                                }
                            }
                        }
                    }
                    _ => return Err(self.error("unexpected '/'")),
                },
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self) -> Result<SpannedValue, ParseError> {
        let span = self.span();
        let kind = match self.peek() {
            None => return Err(self.error("expected a value, found end of file")),
            Some('{') => self.object()?,
            Some('[') => self.array()?,
            Some('"') | Some('\'') => ValueKind::String(self.string()?),
            Some(c) if c == '-' || c == '+' || c == '.' || c.is_ascii_digit() => self.number()?,
            Some(c) if is_ident_start(c) => {
                let word = self.identifier();
                match word.as_str() {
                    "true" => ValueKind::Bool(true),
                    "false" => ValueKind::Bool(false),
                    "null" => ValueKind::Null,
                    "Infinity" | "NaN" => {
                        return Err(self.error_at(
                            span,
                            format!("{word} is not allowed in content (non-finite number)"),
                        ));
                    }
                    other => {
                        return Err(self.error_at(span, format!("unexpected identifier {other:?}")));
                    }
                }
            }
            Some(c) => return Err(self.error(format!("unexpected {}", describe(c)))),
        };
        Ok(SpannedValue { span, kind })
    }

    fn object(&mut self) -> Result<ValueKind, ParseError> {
        self.expect('{', "'{'")?;
        let mut entries: Vec<(Key, SpannedValue)> = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.eat('}') {
                break;
            }
            let key_span = self.span();
            let name = match self.peek() {
                Some('"') | Some('\'') => self.string()?,
                Some(c) if is_ident_start(c) || c == '\\' => self.identifier(),
                Some(c) => {
                    return Err(
                        self.error(format!("expected a key or '}}', found {}", describe(c)))
                    );
                }
                None => return Err(self.error("expected a key or '}', found end of file")),
            };
            if name.is_empty() {
                return Err(self.error_at(key_span, "empty key"));
            }
            if let Some((first, _)) = entries.iter().find(|(k, _)| k.name == name) {
                return Err(self.error_at(
                    key_span,
                    format!(
                        "duplicate key {name:?} (first at {}:{})",
                        first.span.line, first.span.col
                    ),
                ));
            }
            self.skip_trivia()?;
            self.expect(':', "':' after the key")?;
            self.skip_trivia()?;
            let value = self.value()?;
            entries.push((
                Key {
                    name,
                    span: key_span,
                },
                value,
            ));
            self.skip_trivia()?;
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    break;
                }
                Some(c) => {
                    return Err(self.error(format!("expected ',' or '}}', found {}", describe(c))));
                }
                None => return Err(self.error("expected ',' or '}', found end of file")),
            }
        }
        Ok(ValueKind::Object(entries))
    }

    fn array(&mut self) -> Result<ValueKind, ParseError> {
        self.expect('[', "'['")?;
        let mut items = Vec::new();
        loop {
            self.skip_trivia()?;
            if self.eat(']') {
                break;
            }
            items.push(self.value()?);
            self.skip_trivia()?;
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some(']') => {
                    self.bump();
                    break;
                }
                Some(c) => {
                    return Err(self.error(format!("expected ',' or ']', found {}", describe(c))));
                }
                None => return Err(self.error("expected ',' or ']', found end of file")),
            }
        }
        Ok(ValueKind::Array(items))
    }

    fn identifier(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if is_ident_part(c) {
                s.push(c);
                self.bump();
            } else {
                break;
            }
        }
        s
    }

    fn hex_digits(&mut self, count: usize, what: &str) -> Result<u32, ParseError> {
        let mut v = 0u32;
        for _ in 0..count {
            let start = self.span();
            match self.bump().and_then(|c| c.to_digit(16)) {
                Some(d) => v = v * 16 + d,
                None => return Err(self.error_at(start, format!("invalid {what} escape"))),
            }
        }
        Ok(v)
    }

    fn string(&mut self) -> Result<String, ParseError> {
        let start = self.span();
        let quote = self.bump().expect("caller saw a quote");
        let mut s = String::new();
        loop {
            let c = match self.peek() {
                None => return Err(self.error_at(start, "unterminated string")),
                Some(c) => c,
            };
            if c == quote {
                self.bump();
                return Ok(s);
            }
            if c == '\n' || c == '\r' {
                return Err(
                    self.error("line break inside a string (use \\ at the end of the line)")
                );
            }
            if c != '\\' {
                s.push(c);
                self.bump();
                continue;
            }
            let esc_span = self.span();
            self.bump();
            let Some(e) = self.bump() else {
                return Err(self.error_at(start, "unterminated string"));
            };
            match e {
                'n' => s.push('\n'),
                't' => s.push('\t'),
                'r' => s.push('\r'),
                'b' => s.push('\u{8}'),
                'f' => s.push('\u{C}'),
                'v' => s.push('\u{B}'),
                '0' if !self.peek().is_some_and(|c| c.is_ascii_digit()) => s.push('\0'),
                'x' => {
                    let v = self.hex_digits(2, "\\x")?;
                    s.push(char::from_u32(v).expect("two hex digits fit a char"));
                }
                'u' => {
                    let hi = self.hex_digits(4, "\\u")?;
                    if (0xD800..0xDC00).contains(&hi) {
                        if self.peek() == Some('\\') && self.peek_at(1) == Some('u') {
                            self.bump();
                            self.bump();
                            let lo = self.hex_digits(4, "\\u")?;
                            if !(0xDC00..0xE000).contains(&lo) {
                                return Err(self.error_at(esc_span, "invalid surrogate pair"));
                            }
                            let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                            s.push(char::from_u32(cp).expect("valid surrogate pair"));
                        } else {
                            return Err(self.error_at(esc_span, "lone high surrogate"));
                        }
                    } else if let Some(c) = char::from_u32(hi) {
                        s.push(c);
                    } else {
                        return Err(self.error_at(esc_span, "lone low surrogate"));
                    }
                }
                '\r' => {
                    // Line continuation; swallow a following LF.
                    self.eat('\n');
                }
                '\n' | '\u{2028}' | '\u{2029}' => {}
                c if c.is_ascii_digit() => {
                    return Err(
                        self.error_at(esc_span, "numeric escapes other than \\0 are not allowed")
                    );
                }
                c => s.push(c),
            }
        }
    }

    fn number(&mut self) -> Result<ValueKind, ParseError> {
        let start = self.span();
        let mut lexeme = String::new();
        if let Some(c @ ('+' | '-')) = self.peek() {
            lexeme.push(c);
            self.bump();
        }
        if self.peek().is_some_and(is_ident_start) {
            let word = self.identifier();
            return Err(self.error_at(
                start,
                format!("{lexeme}{word} is not allowed in content (non-finite number)"),
            ));
        }
        if self.peek() == Some('0') && matches!(self.peek_at(1), Some('x' | 'X')) {
            self.bump();
            self.bump();
            let mut digits = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    digits.push(c);
                    self.bump();
                } else {
                    break;
                }
            }
            if digits.is_empty() {
                return Err(self.error_at(start, "hex number without digits"));
            }
            let negative = lexeme.starts_with('-');
            return match i64::from_str_radix(&digits, 16) {
                Ok(v) => Ok(ValueKind::Number(Num::Int(if negative { -v } else { v }))),
                Err(_) => Err(self.error_at(start, "hex number does not fit 64 bits")),
            };
        }
        let mut is_int = true;
        let mut saw_digit = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                saw_digit = true;
            } else if c == '.' {
                if !is_int || lexeme.contains(['e', 'E']) {
                    return Err(self.error("unexpected '.' in number"));
                }
                is_int = false;
            } else if c == 'e' || c == 'E' {
                if !saw_digit || lexeme.contains(['e', 'E']) {
                    return Err(self.error("malformed exponent"));
                }
                is_int = false;
                lexeme.push(c);
                self.bump();
                if let Some(s @ ('+' | '-')) = self.peek() {
                    lexeme.push(s);
                    self.bump();
                }
                if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    return Err(self.error("malformed exponent"));
                }
                continue;
            } else {
                break;
            }
            lexeme.push(c);
            self.bump();
        }
        if !saw_digit {
            return Err(self.error_at(start, "malformed number"));
        }
        if self.peek().is_some_and(is_ident_part) {
            return Err(self.error("unexpected character after number"));
        }
        if is_int && let Ok(v) = lexeme.parse::<i64>() {
            return Ok(ValueKind::Number(Num::Int(v)));
        }
        match lexeme.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(ValueKind::Number(Num::Float(f))),
            _ => Err(self.error_at(start, "number out of range")),
        }
    }
}
