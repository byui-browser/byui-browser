//! Lexical analysis: converts source text into [`Token`]s with byte spans.

use std::iter::Peekable;
use std::str::CharIndices;

type Chars<'source> = Peekable<CharIndices<'source>>;

/// A token produced by the JavaScript lexer.
#[derive(Debug, PartialEq)]
pub enum Token {
    /// Numeric literal parsed as a 64-bit float.
    Number(f64),
    /// String literal after supported escapes have been decoded.
    String(String),
    /// Identifier that is not a reserved keyword, with any `\u` escapes
    /// decoded, so `caf\u{E9}` and `café` are both `Identifier("café")`.
    Identifier(String),
    /// Keyword spelled with at least one `\u` escape, such as `\u0069f`,
    /// holding the decoded keyword text (`"if"`).
    ///
    /// ECMAScript forbids escaped keywords everywhere except where any
    /// `IdentifierName` is allowed, so the parser accepts this token only as a
    /// property name after `.` and reports a syntax error anywhere else. This
    /// keeps `var \u0069f = 1` from declaring a variable named `if`.
    EscapedKeyword(String),
    /// `+` operator.
    Plus,
    /// `-` operator.
    Subtract,
    /// `*` operator.
    Multiply,
    /// `/` operator.
    Divide,
    /// `%` operator.
    Remainder,
    /// `let` keyword.
    Let,
    /// `var` keyword.
    Var,
    /// `const` keyword.
    Const,
    /// `if` keyword.
    If,
    /// `else` keyword.
    Else,
    /// `while` keyword.
    While,
    /// `function` keyword.
    Function,
    /// `return` keyword.
    Return,
    /// `class` keyword.
    Class,
    /// `extends` keyword.
    Extends,
    /// `new` keyword.
    New,
    /// `this` keyword.
    This,
    /// `super` keyword.
    Super,
    /// `true` literal.
    True,
    /// `false` literal.
    False,
    /// `null` literal.
    Null,
    /// `undefined` literal (included as a Sprint 2 convenience).
    Undefined,
    /// `=` assignment operator.
    Assign,
    /// `==` equality operator.
    EqualEqual,
    /// `===` strict equality operator.
    StrictEqual,
    /// `!=` inequality operator.
    BangEqual,
    /// `!==` strict inequality operator.
    StrictBangEqual,
    /// `<` comparison operator.
    LessThan,
    /// `<=` comparison operator.
    LessEqual,
    /// `>` comparison operator.
    GreaterThan,
    /// `>=` comparison operator.
    GreaterEqual,
    /// `!` unary negation operator.
    Bang,
    /// `&&` logical AND operator.
    AndAnd,
    /// `||` logical OR operator.
    OrOr,
    /// `&` bitwise AND operator.
    BitAnd,
    /// `|` bitwise OR operator.
    BitOr,
    /// `^` bitwise XOR operator.
    BitXor,
    /// `~` bitwise NOT operator.
    BitNot,
    /// `<<` left shift operator.
    ShiftLeft,
    /// `>>` sign-propagating right shift operator.
    ShiftRight,
    /// `>>>` zero-fill right shift operator.
    UnsignedShiftRight,
    /// `;` statement terminator.
    Semicolon,
    /// `(` delimiter.
    LeftParen,
    /// `)` delimiter.
    RightParen,
    /// `{` block opener.
    LeftBrace,
    /// `}` block closer.
    RightBrace,
    /// `,` separator.
    Comma,
    /// `.` member access operator.
    Dot,
    /// `[` computed member access opener.
    LeftBracket,
    /// `]` computed member access closer.
    RightBracket,
    /// Unrecognized source character. Kept in the stream so callers can
    /// report unsupported syntax without the lexer silently dropping it.
    Unknown(char),
    /// Malformed number, string, or comment, with a human-readable reason.
    Invalid(String),
}

/// A token and its UTF-8 byte range in the original source.
#[derive(Debug, PartialEq)]
pub struct SpannedToken {
    /// The lexical token owned by this entry.
    pub token: Token,
    /// Inclusive byte offset of the token's first character.
    pub start: usize,
    /// Exclusive byte offset after the token's last character.
    pub end: usize,
}

/// Converts source text into tokens.
///
/// Whitespace and line terminators follow the ECMAScript definitions, and
/// `//` line comments and `/* */` block comments are skipped. An unterminated
/// block comment produces [`Token::Invalid`].
///
/// Strings may use single or double quotes. Supported escapes are `\n`, `\r`,
/// `\t`, `\b`, `\f`, `\v`, `\0` (when not followed by a digit), `\xHH`,
/// `\uHHHH`, `\u{H...}`, line continuations, and identity escapes such as
/// `\'` or `\q`. Surrogate-pair `\u` escapes are combined into one character.
///
/// Limitations: legacy octal escapes (`\1`, `\01`, ...), `\8`, `\9`, and lone
/// surrogate escapes are unsupported because Rust strings cannot hold
/// unpaired surrogates. These, malformed hexadecimal escapes, raw line breaks
/// in strings, unterminated strings, and malformed numeric literals produce
/// [`Token::Invalid`]. Other unsupported characters produce
/// [`Token::Unknown`]. This lexer intentionally returns tokens rather than
/// failing fast, so callers can inspect all input.
///
/// Identifiers follow ECMAScript `IdentifierName`: they start with a Unicode
/// `ID_Start` character, `$`, or `_`, and continue with `ID_Continue`
/// characters, `$`, U+200C (ZWNJ), or U+200D (ZWJ). Any of these characters
/// may be written as a `\uHHHH` or `\u{H...}` escape, which is decoded, so an
/// escaped and an unescaped spelling name the same binding. A malformed
/// escape, a non-`\u` escape, or an escape whose character is not allowed at
/// its position makes the whole identifier one [`Token::Invalid`]. Keywords
/// written with escapes produce [`Token::EscapedKeyword`]. A raw character
/// that cannot start an identifier still produces [`Token::Unknown`].
///
/// Limitations: identifiers are not Unicode-normalized, so precomposed `é`
/// and `e` followed by a combining acute accent are different names, and no
/// confusable-character policy is applied. Character classes come from
/// Unicode `XID_Start`/`XID_Continue`, which omit a few dozen compatibility
/// characters that `ID_Start`/`ID_Continue` include (such as U+309B and
/// U+FF9E), so those are rejected. Escapes cannot encode surrogates, matching
/// ECMAScript, which never allows surrogates in identifiers.
pub fn tokenize(input: &str) -> Vec<Token> {
    tokenize_spanned(input)
        .into_iter()
        .map(|entry| entry.token)
        .collect()
}

/// Tokenizes source while retaining UTF-8 byte ranges for diagnostics.
///
/// Uses the same lexical rules and error tokens as [`tokenize`].
pub fn tokenize_spanned(input: &str) -> Vec<SpannedToken> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((start, ch)) = chars.peek().copied() {
        if is_js_whitespace(ch) {
            chars.next();
            continue;
        }

        if ch == '/' {
            match chars.clone().nth(1).map(|(_, next)| next) {
                Some('/') => {
                    skip_line_comment(&mut chars);
                    continue;
                }
                Some('*') => {
                    if !skip_block_comment(&mut chars) {
                        tokens.push(SpannedToken {
                            token: Token::Invalid("unterminated block comment".into()),
                            start,
                            end: input.len(),
                        });
                    }
                    continue;
                }
                _ => {}
            }
        }

        let token = if ch.is_ascii_digit()
            || (ch == '.'
                && chars
                    .clone()
                    .nth(1)
                    .is_some_and(|(_, next)| next.is_ascii_digit()))
        {
            scan_number(input, &mut chars)
        } else if ch == '\'' || ch == '"' {
            scan_string(&mut chars, ch)
        } else if is_identifier_start(ch) || ch == '\\' {
            scan_identifier(&mut chars)
        } else {
            chars.next();
            match ch {
                '+' => Token::Plus,
                '-' => Token::Subtract,
                '*' => Token::Multiply,
                '/' => Token::Divide,
                '%' => Token::Remainder,
                '=' => match take_if(&mut chars, '=') {
                    true if take_if(&mut chars, '=') => Token::StrictEqual,
                    true => Token::EqualEqual,
                    false => Token::Assign,
                },
                '!' => match take_if(&mut chars, '=') {
                    true if take_if(&mut chars, '=') => Token::StrictBangEqual,
                    true => Token::BangEqual,
                    false => Token::Bang,
                },
                '<' if take_if(&mut chars, '<') => Token::ShiftLeft,
                '<' if take_if(&mut chars, '=') => Token::LessEqual,
                '<' => Token::LessThan,
                '>' if take_if(&mut chars, '>') => {
                    if take_if(&mut chars, '>') {
                        Token::UnsignedShiftRight
                    } else {
                        Token::ShiftRight
                    }
                }
                '>' if take_if(&mut chars, '=') => Token::GreaterEqual,
                '>' => Token::GreaterThan,
                '&' if take_if(&mut chars, '&') => Token::AndAnd,
                '&' => Token::BitAnd,
                '|' if take_if(&mut chars, '|') => Token::OrOr,
                '|' => Token::BitOr,
                '^' => Token::BitXor,
                '~' => Token::BitNot,
                ';' => Token::Semicolon,
                '(' => Token::LeftParen,
                ')' => Token::RightParen,
                '{' => Token::LeftBrace,
                '}' => Token::RightBrace,
                ',' => Token::Comma,
                '.' => Token::Dot,
                '[' => Token::LeftBracket,
                ']' => Token::RightBracket,
                other => Token::Unknown(other),
            }
        };
        let end = chars.peek().map_or(input.len(), |(index, _)| *index);
        tokens.push(SpannedToken { token, start, end });
    }

    tokens
}

/// Returns whether `ch` is ECMAScript `WhiteSpace` or a `LineTerminator`.
pub(crate) fn is_js_whitespace(ch: char) -> bool {
    matches!(
        ch,
        '\t' | '\u{000B}' | '\u{000C}' | ' ' | '\u{00A0}' | '\u{FEFF}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    ) || is_line_terminator(ch)
}

fn is_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Skips a `//` comment up to, but not including, the next line terminator.
fn skip_line_comment(chars: &mut Chars<'_>) {
    while chars.next_if(|(_, ch)| !is_line_terminator(*ch)).is_some() {}
}

/// Skips a `/* */` comment, returning `false` if it is never closed.
fn skip_block_comment(chars: &mut Chars<'_>) -> bool {
    chars.next(); // '/'
    chars.next(); // '*'
    while let Some((_, ch)) = chars.next() {
        if ch == '*' && take_if(chars, '/') {
            return true;
        }
    }
    false
}

fn scan_number(input: &str, chars: &mut Chars<'_>) -> Token {
    let start = chars.peek().expect("number begins with a character").0;
    let mut end = start;
    let mut seen_decimal = false;
    let mut seen_exponent = false;
    let mut exponent_needs_digit = false;
    let mut malformed = false;

    while let Some((index, ch)) = chars.peek().copied() {
        if ch.is_ascii_digit() {
            end = index + ch.len_utf8();
            chars.next();
            exponent_needs_digit = false;
        } else if ch == '.' {
            end = index + ch.len_utf8();
            chars.next();
            if seen_decimal || seen_exponent {
                malformed = true;
            }
            seen_decimal = true;
        } else if (ch == 'e' || ch == 'E') && !seen_exponent {
            end = index + ch.len_utf8();
            chars.next();
            seen_exponent = true;
            exponent_needs_digit = true;
            if matches!(chars.peek(), Some((_, '+' | '-'))) {
                let (sign_index, sign) = chars.next().expect("peeked exponent sign exists");
                end = sign_index + sign.len_utf8();
            }
        } else {
            break;
        }
    }

    let number_text = &input[start..end];
    if malformed || exponent_needs_digit {
        Token::Invalid(format!("malformed number {number_text:?}"))
    } else {
        number_text.parse::<f64>().map_or_else(
            |_| Token::Invalid(format!("malformed number {number_text:?}")),
            Token::Number,
        )
    }
}

const UNSUPPORTED_ESCAPE: &str = "string contains an unsupported escape";
const INVALID_ESCAPE: &str = "string contains an invalid escape";

fn scan_string(chars: &mut Chars<'_>, quote: char) -> Token {
    chars.next(); // opening quote
    let mut value = String::new();
    let mut error: Option<&'static str> = None;

    while let Some((_, ch)) = chars.next() {
        if ch == quote {
            return match error {
                Some(message) => Token::Invalid(message.into()),
                None => Token::String(value),
            };
        }

        if ch == '\n' || ch == '\r' {
            return Token::Invalid("string literal contains a line break".into());
        }

        if ch != '\\' {
            value.push(ch);
            continue;
        }

        let Some((_, escape)) = chars.next() else {
            return Token::Invalid("unterminated string literal".into());
        };
        let decoded = match escape {
            'n' => Ok(Some('\n')),
            '\r' => {
                take_if(chars, '\n');
                Ok(None)
            }
            '\n' | '\u{2028}' | '\u{2029}' => Ok(None),
            'r' => Ok(Some('\r')),
            't' => Ok(Some('\t')),
            'b' => Ok(Some('\u{0008}')),
            'f' => Ok(Some('\u{000C}')),
            'v' => Ok(Some('\u{000B}')),
            '0' if !chars.peek().is_some_and(|(_, next)| next.is_ascii_digit()) => Ok(Some('\0')),
            '0'..='9' => Err(UNSUPPORTED_ESCAPE),
            'x' => read_hex_digits(chars, 2)
                .and_then(char::from_u32)
                .map(Some)
                .ok_or(INVALID_ESCAPE),
            'u' => read_unicode_escape(chars).map(Some),
            other => Ok(Some(other)),
        };
        match decoded {
            Ok(Some(decoded)) => value.push(decoded),
            Ok(None) => {}
            Err(message) => {
                error.get_or_insert(message);
            }
        }
    }

    Token::Invalid("unterminated string literal".into())
}

/// Decodes the body of a `\u` escape (the `u` already consumed), combining a
/// high surrogate with an immediately following low-surrogate `\u` escape.
fn read_unicode_escape(chars: &mut Chars<'_>) -> Result<char, &'static str> {
    let code_unit = read_unicode_code_point(chars).ok_or(INVALID_ESCAPE)?;
    if !(0xD800..=0xDBFF).contains(&code_unit) {
        return char::from_u32(code_unit).ok_or(UNSUPPORTED_ESCAPE);
    }

    let mut lookahead = chars.clone();
    if take_if(&mut lookahead, '\\') && take_if(&mut lookahead, 'u') {
        if let Some(low @ 0xDC00..=0xDFFF) = read_unicode_code_point(&mut lookahead) {
            *chars = lookahead;
            let combined = 0x10000 + ((code_unit - 0xD800) << 10) + (low - 0xDC00);
            return char::from_u32(combined).ok_or(INVALID_ESCAPE);
        }
    }
    Err(UNSUPPORTED_ESCAPE)
}

/// Reads `HHHH` or `{H...}` after `\u`, returning the code point value. A
/// braced escape is consumed through its `}` even when its value is out of
/// range, so the brace is not left behind as a separate token.
fn read_unicode_code_point(chars: &mut Chars<'_>) -> Option<u32> {
    if !take_if(chars, '{') {
        return read_hex_digits(chars, 4);
    }
    let mut value: u32 = 0;
    let mut digits = 0;
    while let Some(digit) = chars.peek().and_then(|(_, ch)| ch.to_digit(16)) {
        chars.next();
        value = value.saturating_mul(16).saturating_add(digit);
        digits += 1;
    }
    let closed = take_if(chars, '}');
    (digits > 0 && closed && value <= 0x10FFFF).then_some(value)
}

/// Reads exactly `count` hexadecimal digits, leaving a non-hex character
/// unconsumed so it is still processed as string content.
fn read_hex_digits(chars: &mut Chars<'_>, count: usize) -> Option<u32> {
    let mut value = 0;
    for _ in 0..count {
        let digit = chars.peek().and_then(|(_, ch)| ch.to_digit(16))?;
        chars.next();
        value = value * 16 + digit;
    }
    Some(value)
}

fn take_if(chars: &mut Chars<'_>, expected: char) -> bool {
    chars.next_if(|(_, ch)| *ch == expected).is_some()
}

/// Scans an identifier or keyword starting at an `ID_Start` character or `\`,
/// decoding `\u` escapes. After an error, scanning continues to the end of
/// the identifier so it becomes a single [`Token::Invalid`].
fn scan_identifier(chars: &mut Chars<'_>) -> Token {
    let mut name = String::new();
    let mut escaped = false;
    let mut error: Option<String> = None;

    while let Some((_, ch)) = chars.peek().copied() {
        let first = name.is_empty() && !escaped;
        if ch != '\\' {
            let allowed = if first {
                is_identifier_start(ch)
            } else {
                is_identifier_continue(ch)
            };
            if !allowed {
                break;
            }
            chars.next();
            name.push(ch);
            continue;
        }

        chars.next(); // '\'
        escaped = true;
        if !take_if(chars, 'u') {
            error.get_or_insert_with(|| "identifier contains an escape other than \\u".into());
            continue;
        }
        let Some(code_point) = read_unicode_code_point(chars) else {
            error.get_or_insert_with(|| "identifier contains a malformed \\u escape".into());
            continue;
        };
        let decoded = char::from_u32(code_point).filter(|&decoded| {
            if first {
                is_identifier_start(decoded)
            } else {
                is_identifier_continue(decoded)
            }
        });
        match decoded {
            Some(decoded) => name.push(decoded),
            None => {
                let position = if first { "start" } else { "continue" };
                error.get_or_insert_with(|| {
                    format!("escaped character U+{code_point:04X} cannot {position} an identifier")
                });
            }
        }
    }

    if let Some(message) = error {
        return Token::Invalid(message);
    }
    match keyword(&name) {
        Some(_) if escaped => Token::EscapedKeyword(name),
        Some(keyword) => keyword,
        None => Token::Identifier(name),
    }
}

/// Returns the keyword token spelled by `name`, if any.
fn keyword(name: &str) -> Option<Token> {
    Some(match name {
        "let" => Token::Let,
        "var" => Token::Var,
        "const" => Token::Const,
        "if" => Token::If,
        "else" => Token::Else,
        "while" => Token::While,
        "function" => Token::Function,
        "return" => Token::Return,
        "class" => Token::Class,
        "extends" => Token::Extends,
        "new" => Token::New,
        "this" => Token::This,
        "super" => Token::Super,
        "true" => Token::True,
        "false" => Token::False,
        "null" => Token::Null,
        "undefined" => Token::Undefined,
        _ => return None,
    })
}

/// ECMAScript `IdentifierStartChar`: `ID_Start`, `$`, or `_`. Uses
/// `XID_Start`, which differs only in a few compatibility characters.
fn is_identifier_start(ch: char) -> bool {
    ch == '$' || ch == '_' || unicode_ident::is_xid_start(ch)
}

/// ECMAScript `IdentifierPartChar`: `ID_Continue`, `$`, ZWNJ, or ZWJ. Uses
/// `XID_Continue`, which differs only in a few compatibility characters.
fn is_identifier_continue(ch: char) -> bool {
    ch == '$' || ch == '\u{200C}' || ch == '\u{200D}' || unicode_ident::is_xid_continue(ch)
}
