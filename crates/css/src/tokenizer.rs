//! CSS tokenizer following CSS Syntax Level 3 §4 ("Tokenization").
//!
//! [`tokenize`] turns stylesheet text into a flat list of [`Token`]s. Every
//! token records the byte range of the source text it was produced from, so
//! later stages can report precise error offsets and reproduce author text
//! (for example declaration values) without re-scanning characters.
//!
//! Comments are consumed by the tokenizer and never produce tokens, matching
//! the specification. Input preprocessing (§3.3) is applied lazily: `\r`,
//! `\r\n`, and form feed count as newlines, and NUL becomes U+FFFD in token
//! values, while spans still index the original, unmodified input.

use std::borrow::Cow;
use std::ops::Range;

/// One CSS token and the source bytes it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> {
    /// What kind of token this is, with its unescaped value.
    pub kind: TokenKind<'a>,
    /// Byte range of the token in the tokenized input. Spans of consecutive
    /// tokens are contiguous except where a comment was skipped.
    pub span: Range<usize>,
}

/// The token kinds of CSS Syntax Level 3 §4.
///
/// String payloads are unescaped values. They borrow from the input when the
/// source text needed no unescaping, and own a new string otherwise.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    /// `<ident-token>`, such as `color` or `--brand`.
    Ident(Cow<'a, str>),
    /// `<function-token>`: an identifier immediately followed by `(`. The
    /// value is the function name without the parenthesis.
    Function(Cow<'a, str>),
    /// `<at-keyword-token>`, such as `@media`. The value excludes the `@`.
    AtKeyword(Cow<'a, str>),
    /// `<hash-token>`, such as `#main` or `#fff`.
    Hash {
        /// The text after `#`.
        value: Cow<'a, str>,
        /// `true` when the value is a valid identifier, so the token can be
        /// an ID selector (`#main`, but not `#123`).
        is_id: bool,
    },
    /// `<string-token>`: a quoted string. The value excludes the quotes.
    String(Cow<'a, str>),
    /// `<bad-string-token>`: a string interrupted by an unescaped newline.
    BadString,
    /// `<url-token>`: an unquoted `url(...)`. The value is the URL text.
    Url(Cow<'a, str>),
    /// `<bad-url-token>`: a malformed unquoted `url(...)`.
    BadUrl,
    /// `<delim-token>`: any single code point without a token of its own.
    Delim(char),
    /// `<number-token>`, such as `1`, `-2.5`, or `1e3`.
    Number {
        /// Numeric value.
        value: f64,
        /// `true` for the "integer" type flag (no `.` or exponent).
        is_integer: bool,
    },
    /// `<percentage-token>`, such as `50%`. The value is the number before `%`.
    Percentage(f64),
    /// `<dimension-token>`, such as `10px` or `1.5em`.
    Dimension {
        /// Numeric value.
        value: f64,
        /// `true` for the "integer" type flag.
        is_integer: bool,
        /// Unit identifier, as written (units compare ASCII case-insensitively).
        unit: Cow<'a, str>,
    },
    /// `<whitespace-token>`: one or more spaces, tabs, or newlines.
    Whitespace,
    /// `<CDO-token>`: `<!--`.
    Cdo,
    /// `<CDC-token>`: `-->`.
    Cdc,
    /// `<colon-token>`: `:`.
    Colon,
    /// `<semicolon-token>`: `;`.
    Semicolon,
    /// `<comma-token>`: `,`.
    Comma,
    /// `<[-token>`.
    OpenSquare,
    /// `<]-token>`.
    CloseSquare,
    /// `<(-token>`.
    OpenParen,
    /// `<)-token>`.
    CloseParen,
    /// `<{-token>`.
    OpenCurly,
    /// `<}-token>`.
    CloseCurly,
}

/// Tokenizes `input` into CSS tokens, skipping comments.
///
/// The tokenizer never fails: malformed input produces the recovery tokens
/// the specification defines (`BadString`, `BadUrl`, `Delim`). The end of
/// the returned vector is the `<EOF-token>`.
pub fn tokenize(input: &str) -> Vec<Token<'_>> {
    let mut tokenizer = Tokenizer {
        input,
        bytes: input.as_bytes(),
        pos: 0,
    };
    // Typical CSS averages roughly one token per four bytes.
    let mut tokens = Vec::with_capacity(input.len() / 4);
    while let Some(token) = tokenizer.next_token() {
        tokens.push(token);
    }
    log::trace!(
        "tokenized {} bytes into {} tokens",
        input.len(),
        tokens.len()
    );
    tokens
}

struct Tokenizer<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

fn is_newline(byte: u8) -> bool {
    matches!(byte, b'\n' | b'\r' | 0x0C)
}

fn is_whitespace(byte: u8) -> bool {
    is_newline(byte) || byte == b' ' || byte == b'\t'
}

/// A name-start code point. Any byte of a non-ASCII UTF-8 sequence is
/// `>= 0x80`, and every non-ASCII code point is a name-start code point, so
/// this byte test is exact. NUL is included because preprocessing maps it to
/// U+FFFD, which is non-ASCII.
fn is_name_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80 || byte == 0
}

fn is_name(byte: u8) -> bool {
    is_name_start(byte) || byte.is_ascii_digit() || byte == b'-'
}

fn is_non_printable(byte: u8) -> bool {
    matches!(byte, 0x00..=0x08 | 0x0B | 0x0E..=0x1F | 0x7F)
}

impl<'a> Tokenizer<'a> {
    fn byte_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    /// Decodes the code point at the cursor without consuming it.
    fn current_char(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    /// Consumes one code point (one to four bytes).
    fn advance_char(&mut self) -> Option<char> {
        let character = self.current_char()?;
        self.pos += character.len_utf8();
        Some(character)
    }

    /// Consumes one newline, treating `\r\n` as a single newline.
    fn consume_newline(&mut self) {
        if self.byte_at(0) == Some(b'\r') && self.byte_at(1) == Some(b'\n') {
            self.pos += 2;
        } else {
            self.pos += 1;
        }
    }

    /// §4.3.8: whether the bytes at `offset` and `offset + 1` start an escape.
    fn is_valid_escape_at(&self, offset: usize) -> bool {
        self.byte_at(offset) == Some(b'\\') && !self.byte_at(offset + 1).is_some_and(is_newline)
    }

    /// §4.3.9: whether the input at `offset` would start an ident sequence.
    fn would_start_ident_at(&self, offset: usize) -> bool {
        match self.byte_at(offset) {
            Some(b'-') => {
                self.byte_at(offset + 1)
                    .is_some_and(|next| is_name_start(next) || next == b'-')
                    || self.is_valid_escape_at(offset + 1)
            }
            Some(b'\\') => self.is_valid_escape_at(offset),
            Some(byte) => is_name_start(byte),
            None => false,
        }
    }

    /// §4.3.10: whether the input at the cursor would start a number.
    fn would_start_number(&self) -> bool {
        match self.byte_at(0) {
            Some(b'+' | b'-') => match self.byte_at(1) {
                Some(digit) if digit.is_ascii_digit() => true,
                Some(b'.') => self.byte_at(2).is_some_and(|byte| byte.is_ascii_digit()),
                _ => false,
            },
            Some(b'.') => self.byte_at(1).is_some_and(|byte| byte.is_ascii_digit()),
            Some(byte) => byte.is_ascii_digit(),
            None => false,
        }
    }

    /// §4.3.2: skips any number of comments at the cursor.
    fn consume_comments(&mut self) {
        while self.byte_at(0) == Some(b'/') && self.byte_at(1) == Some(b'*') {
            match self.input[self.pos + 2..].find("*/") {
                Some(end) => self.pos += 2 + end + 2,
                None => {
                    log::trace!("unterminated css comment at offset {}", self.pos);
                    self.pos = self.bytes.len();
                }
            }
        }
    }

    fn next_token(&mut self) -> Option<Token<'a>> {
        self.consume_comments();
        let start = self.pos;
        let byte = self.byte_at(0)?;
        let kind = match byte {
            byte if is_whitespace(byte) => {
                while self.byte_at(0).is_some_and(is_whitespace) {
                    self.pos += 1;
                }
                TokenKind::Whitespace
            }
            b'"' | b'\'' => {
                self.pos += 1;
                self.consume_string(byte)
            }
            b'#' => {
                if self.byte_at(1).is_some_and(is_name) || self.is_valid_escape_at(1) {
                    self.pos += 1;
                    let is_id = self.would_start_ident_at(0);
                    TokenKind::Hash {
                        value: self.consume_ident_sequence(),
                        is_id,
                    }
                } else {
                    self.pos += 1;
                    TokenKind::Delim('#')
                }
            }
            b'(' => self.single(TokenKind::OpenParen),
            b')' => self.single(TokenKind::CloseParen),
            b'[' => self.single(TokenKind::OpenSquare),
            b']' => self.single(TokenKind::CloseSquare),
            b'{' => self.single(TokenKind::OpenCurly),
            b'}' => self.single(TokenKind::CloseCurly),
            b',' => self.single(TokenKind::Comma),
            b':' => self.single(TokenKind::Colon),
            b';' => self.single(TokenKind::Semicolon),
            b'+' | b'.' => {
                if self.would_start_number() {
                    self.consume_numeric()
                } else {
                    self.pos += 1;
                    TokenKind::Delim(byte as char)
                }
            }
            b'-' => {
                if self.would_start_number() {
                    self.consume_numeric()
                } else if self.byte_at(1) == Some(b'-') && self.byte_at(2) == Some(b'>') {
                    self.pos += 3;
                    TokenKind::Cdc
                } else if self.would_start_ident_at(0) {
                    self.consume_ident_like()
                } else {
                    self.pos += 1;
                    TokenKind::Delim('-')
                }
            }
            b'<' => {
                if self.bytes[self.pos..].starts_with(b"<!--") {
                    self.pos += 4;
                    TokenKind::Cdo
                } else {
                    self.pos += 1;
                    TokenKind::Delim('<')
                }
            }
            b'@' => {
                if self.would_start_ident_at(1) {
                    self.pos += 1;
                    TokenKind::AtKeyword(self.consume_ident_sequence())
                } else {
                    self.pos += 1;
                    TokenKind::Delim('@')
                }
            }
            b'\\' => {
                if self.is_valid_escape_at(0) {
                    self.consume_ident_like()
                } else {
                    log::trace!("invalid css escape at offset {}", self.pos);
                    self.pos += 1;
                    TokenKind::Delim('\\')
                }
            }
            byte if byte.is_ascii_digit() => self.consume_numeric(),
            byte if is_name_start(byte) => self.consume_ident_like(),
            _ => TokenKind::Delim(self.advance_char().unwrap_or('\u{FFFD}')),
        };
        Some(Token {
            kind,
            span: start..self.pos,
        })
    }

    fn single(&mut self, kind: TokenKind<'a>) -> TokenKind<'a> {
        self.pos += 1;
        kind
    }

    /// §4.3.7: consumes an escape whose `\` has already been consumed.
    fn consume_escape(&mut self) -> char {
        let Some(byte) = self.byte_at(0) else {
            return '\u{FFFD}';
        };
        if byte.is_ascii_hexdigit() {
            let start = self.pos;
            while self.pos - start < 6 && self.byte_at(0).is_some_and(|b| b.is_ascii_hexdigit()) {
                self.pos += 1;
            }
            let value = u32::from_str_radix(&self.input[start..self.pos], 16).unwrap_or(0);
            if self.byte_at(0).is_some_and(is_whitespace) {
                self.consume_newline_or_space();
            }
            match char::from_u32(value) {
                Some(character) if value != 0 => character,
                _ => '\u{FFFD}',
            }
        } else {
            match self.advance_char() {
                Some('\0') | None => '\u{FFFD}',
                Some(character) => character,
            }
        }
    }

    fn consume_newline_or_space(&mut self) {
        if self.byte_at(0).is_some_and(is_newline) {
            self.consume_newline();
        } else {
            self.pos += 1;
        }
    }

    /// §4.3.11: consumes an ident sequence, borrowing when nothing needs
    /// unescaping.
    fn consume_ident_sequence(&mut self) -> Cow<'a, str> {
        let start = self.pos;
        while let Some(byte) = self.byte_at(0) {
            if byte == 0 || byte == b'\\' {
                break;
            }
            if !is_name(byte) {
                return Cow::Borrowed(&self.input[start..self.pos]);
            }
            self.pos += 1;
        }
        if self.pos >= self.bytes.len() {
            return Cow::Borrowed(&self.input[start..self.pos]);
        }
        // Slow path: an escape or NUL needs an owned value.
        let mut value = self.input[start..self.pos].to_owned();
        loop {
            match self.byte_at(0) {
                Some(0) => {
                    self.pos += 1;
                    value.push('\u{FFFD}');
                }
                Some(b'\\') if self.is_valid_escape_at(0) => {
                    self.pos += 1;
                    value.push(self.consume_escape());
                }
                Some(byte) if is_name(byte) => {
                    let character = self.advance_char().unwrap_or('\u{FFFD}');
                    value.push(character);
                }
                _ => return Cow::Owned(value),
            }
        }
    }

    /// §4.3.12: consumes a number and reports its value and type flag.
    fn consume_number(&mut self) -> (f64, bool) {
        let start = self.pos;
        let mut is_integer = true;
        if matches!(self.byte_at(0), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        self.skip_digits();
        if self.byte_at(0) == Some(b'.') && self.byte_at(1).is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
            self.skip_digits();
            is_integer = false;
        }
        if matches!(self.byte_at(0), Some(b'e' | b'E')) {
            let digit_offset = if matches!(self.byte_at(1), Some(b'+' | b'-')) {
                2
            } else {
                1
            };
            if self
                .byte_at(digit_offset)
                .is_some_and(|b| b.is_ascii_digit())
            {
                self.pos += digit_offset;
                self.skip_digits();
                is_integer = false;
            }
        }
        let value = self.input[start..self.pos].parse::<f64>().unwrap_or(0.0);
        (value, is_integer)
    }

    fn skip_digits(&mut self) {
        while self.byte_at(0).is_some_and(|byte| byte.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    /// §4.3.3: consumes a number, percentage, or dimension.
    fn consume_numeric(&mut self) -> TokenKind<'a> {
        let (value, is_integer) = self.consume_number();
        if self.would_start_ident_at(0) {
            TokenKind::Dimension {
                value,
                is_integer,
                unit: self.consume_ident_sequence(),
            }
        } else if self.byte_at(0) == Some(b'%') {
            self.pos += 1;
            TokenKind::Percentage(value)
        } else {
            TokenKind::Number { value, is_integer }
        }
    }

    /// §4.3.4: consumes an ident, function, or url token.
    fn consume_ident_like(&mut self) -> TokenKind<'a> {
        let name = self.consume_ident_sequence();
        if self.byte_at(0) != Some(b'(') {
            return TokenKind::Ident(name);
        }
        self.pos += 1;
        if !name.eq_ignore_ascii_case("url") {
            return TokenKind::Function(name);
        }
        // Leave at most one whitespace token before a quoted URL argument.
        while self.byte_at(0).is_some_and(is_whitespace)
            && self.byte_at(1).is_some_and(is_whitespace)
        {
            self.pos += 1;
        }
        let quote_at = |offset| matches!(self.byte_at(offset), Some(b'"' | b'\''));
        if quote_at(0) || (self.byte_at(0).is_some_and(is_whitespace) && quote_at(1)) {
            TokenKind::Function(name)
        } else {
            self.consume_url()
        }
    }

    /// §4.3.5: consumes a string token after its opening quote.
    fn consume_string(&mut self, quote: u8) -> TokenKind<'a> {
        let start = self.pos;
        let mut owned: Option<String> = None;
        loop {
            let Some(byte) = self.byte_at(0) else {
                log::trace!("unterminated css string at offset {start}");
                break;
            };
            match byte {
                byte if byte == quote => {
                    let end = self.pos;
                    self.pos += 1;
                    return TokenKind::String(match owned {
                        Some(value) => Cow::Owned(value),
                        None => Cow::Borrowed(&self.input[start..end]),
                    });
                }
                byte if is_newline(byte) => {
                    log::trace!("newline inside css string at offset {}", self.pos);
                    return TokenKind::BadString;
                }
                b'\\' => {
                    let value = owned.get_or_insert_with(|| self.input[start..self.pos].to_owned());
                    self.pos += 1;
                    match self.byte_at(0) {
                        None => {}
                        Some(next) if is_newline(next) => self.consume_newline(),
                        Some(_) => value.push(self.consume_escape()),
                    }
                }
                0 => {
                    owned
                        .get_or_insert_with(|| self.input[start..self.pos].to_owned())
                        .push('\u{FFFD}');
                    self.pos += 1;
                }
                _ => {
                    let character = self.advance_char().unwrap_or('\u{FFFD}');
                    if let Some(value) = owned.as_mut() {
                        value.push(character);
                    }
                }
            }
        }
        TokenKind::String(match owned {
            Some(value) => Cow::Owned(value),
            None => Cow::Borrowed(&self.input[start..self.pos]),
        })
    }

    /// §4.3.6: consumes an unquoted URL after `url(`.
    fn consume_url(&mut self) -> TokenKind<'a> {
        while self.byte_at(0).is_some_and(is_whitespace) {
            self.pos += 1;
        }
        let mut value = String::new();
        loop {
            let Some(byte) = self.byte_at(0) else {
                return TokenKind::Url(Cow::Owned(value));
            };
            match byte {
                b')' => {
                    self.pos += 1;
                    return TokenKind::Url(Cow::Owned(value));
                }
                byte if is_whitespace(byte) => {
                    while self.byte_at(0).is_some_and(is_whitespace) {
                        self.pos += 1;
                    }
                    match self.byte_at(0) {
                        Some(b')') => {
                            self.pos += 1;
                            return TokenKind::Url(Cow::Owned(value));
                        }
                        None => return TokenKind::Url(Cow::Owned(value)),
                        Some(_) => return self.consume_bad_url(),
                    }
                }
                b'"' | b'\'' | b'(' => return self.consume_bad_url(),
                byte if is_non_printable(byte) => return self.consume_bad_url(),
                b'\\' => {
                    if self.is_valid_escape_at(0) {
                        self.pos += 1;
                        value.push(self.consume_escape());
                    } else {
                        return self.consume_bad_url();
                    }
                }
                _ => value.push(self.advance_char().unwrap_or('\u{FFFD}')),
            }
        }
    }

    /// §4.3.14: consumes the remnants of a bad URL.
    fn consume_bad_url(&mut self) -> TokenKind<'a> {
        log::trace!("bad css url at offset {}", self.pos);
        while let Some(byte) = self.byte_at(0) {
            if byte == b')' {
                self.pos += 1;
                break;
            }
            if self.is_valid_escape_at(0) {
                self.pos += 1;
                self.consume_escape();
            } else {
                self.advance_char();
            }
        }
        TokenKind::BadUrl
    }
}

/// Reproduces the author text of `tokens` from `source`, without comments.
///
/// Token spans are copied verbatim. Where a comment separated two
/// non-whitespace tokens, a single space is inserted so they do not fuse into
/// one token (`1px/**/2px` serializes as `1px 2px`). The result is trimmed.
pub(crate) fn source_text(source: &str, tokens: &[Token<'_>]) -> String {
    let tokens = trim_whitespace(tokens);
    let (Some(first), Some(last)) = (tokens.first(), tokens.last()) else {
        return String::new();
    };
    // The common case has no comments inside the range: one slice copy.
    let contiguous = tokens
        .windows(2)
        .all(|pair| pair[0].span.end == pair[1].span.start);
    if contiguous {
        return source[first.span.start..last.span.end].to_owned();
    }
    let mut text = String::with_capacity(last.span.end - first.span.start);
    for (index, token) in tokens.iter().enumerate() {
        if index > 0 {
            let previous = &tokens[index - 1];
            if previous.span.end != token.span.start
                && previous.kind != TokenKind::Whitespace
                && token.kind != TokenKind::Whitespace
            {
                text.push(' ');
            }
        }
        text.push_str(&source[token.span.clone()]);
    }
    text
}

/// Returns `tokens` without leading and trailing whitespace tokens.
pub(crate) fn trim_whitespace<'t, 'a>(tokens: &'t [Token<'a>]) -> &'t [Token<'a>] {
    let start = tokens
        .iter()
        .position(|token| token.kind != TokenKind::Whitespace)
        .unwrap_or(tokens.len());
    let end = tokens
        .iter()
        .rposition(|token| token.kind != TokenKind::Whitespace)
        .map_or(start, |index| index + 1);
    &tokens[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(input: &str) -> Vec<TokenKind<'_>> {
        tokenize(input)
            .into_iter()
            .map(|token| token.kind)
            .collect()
    }

    #[test]
    fn tokenizes_rule_punctuation_and_skips_comments() {
        assert_eq!(
            kinds("p/* x */{color:red;}"),
            vec![
                TokenKind::Ident("p".into()),
                TokenKind::OpenCurly,
                TokenKind::Ident("color".into()),
                TokenKind::Colon,
                TokenKind::Ident("red".into()),
                TokenKind::Semicolon,
                TokenKind::CloseCurly,
            ]
        );
    }

    #[test]
    fn tokenizes_numbers_percentages_and_dimensions() {
        assert_eq!(
            kinds("10px -.5em 50% +3 1e2"),
            vec![
                TokenKind::Dimension {
                    value: 10.0,
                    is_integer: true,
                    unit: "px".into()
                },
                TokenKind::Whitespace,
                TokenKind::Dimension {
                    value: -0.5,
                    is_integer: false,
                    unit: "em".into()
                },
                TokenKind::Whitespace,
                TokenKind::Percentage(50.0),
                TokenKind::Whitespace,
                TokenKind::Number {
                    value: 3.0,
                    is_integer: true
                },
                TokenKind::Whitespace,
                TokenKind::Number {
                    value: 100.0,
                    is_integer: false
                },
            ]
        );
    }

    #[test]
    fn distinguishes_id_hashes_from_other_hashes() {
        assert_eq!(
            kinds("#main #123"),
            vec![
                TokenKind::Hash {
                    value: "main".into(),
                    is_id: true
                },
                TokenKind::Whitespace,
                TokenKind::Hash {
                    value: "123".into(),
                    is_id: false
                },
            ]
        );
    }

    #[test]
    fn unescapes_identifiers_and_strings() {
        assert_eq!(
            kinds(r#".\31 23item "a\"b""#),
            vec![
                TokenKind::Delim('.'),
                TokenKind::Ident("123item".into()),
                TokenKind::Whitespace,
                TokenKind::String("a\"b".into()),
            ]
        );
    }

    #[test]
    fn tokenizes_urls_functions_and_bad_strings() {
        assert_eq!(
            kinds("url( a.png ) url(\"b.png\") rgb("),
            vec![
                TokenKind::Url("a.png".into()),
                TokenKind::Whitespace,
                TokenKind::Function("url".into()),
                TokenKind::String("b.png".into()),
                TokenKind::CloseParen,
                TokenKind::Whitespace,
                TokenKind::Function("rgb".into()),
            ]
        );
        assert_eq!(kinds("url(a b)"), vec![TokenKind::BadUrl]);
        assert_eq!(
            kinds("'a\nb'"),
            vec![
                TokenKind::BadString,
                TokenKind::Whitespace,
                TokenKind::Ident("b".into()),
                TokenKind::String("".into()),
            ]
        );
    }

    #[test]
    fn tokenizes_cdo_cdc_at_keywords_and_custom_properties() {
        assert_eq!(
            kinds("<!-- @media --x -->"),
            vec![
                TokenKind::Cdo,
                TokenKind::Whitespace,
                TokenKind::AtKeyword("media".into()),
                TokenKind::Whitespace,
                TokenKind::Ident("--x".into()),
                TokenKind::Whitespace,
                TokenKind::Cdc,
            ]
        );
    }

    #[test]
    fn spans_cover_source_text_and_handle_non_ascii() {
        let input = "é{a:b}";
        let tokens = tokenize(input);
        assert_eq!(tokens[0].kind, TokenKind::Ident("é".into()));
        assert_eq!(&input[tokens[0].span.clone()], "é");
        assert_eq!(tokens.last().unwrap().span.end, input.len());
    }

    #[test]
    fn source_text_drops_comments_without_fusing_tokens() {
        let input = "1px/**/2px  /* x */ 3px";
        let tokens = tokenize(input);
        assert_eq!(source_text(input, &tokens), "1px 2px   3px");
    }
}
