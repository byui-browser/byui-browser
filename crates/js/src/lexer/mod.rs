/// A token produced by the JavaScript lexer.
#[derive(Debug, PartialEq)]
pub enum Token {
    /// Numeric literal parsed as a 64-bit float.
    Number(f64),
    /// String literal after supported escapes have been decoded.
    String(String),
    /// Identifier spelling that is not a reserved keyword.
    Identifier(String),
    /// `+` operator.
    Plus,
    /// `-` operator.
    Subtract,
    /// `*` operator.
    Multiply,
    /// `/` operator.
    Divide,
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
    /// Unrecognized source character. Kept in the stream so callers can
    /// report unsupported syntax without the lexer silently dropping it.
    Unknown(char),
    /// Malformed number or string, with a human-readable reason.
    Invalid(String),
}

/// Converts source text into tokens.
///
/// Strings may use single or double quotes. The supported escapes are `\\`,
/// `\'`, `\"`, `\n`, `\r`, and `\t`; these are decoded into the corresponding
/// character. Unsupported escapes, raw line breaks in strings, unterminated
/// strings, and malformed numeric literals produce [`Token::Invalid`]. Other
/// unsupported characters produce [`Token::Unknown`]. This lexer intentionally
/// returns tokens rather than failing fast, so callers can inspect all input.
pub fn tokenize(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();

    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        if ch.is_ascii_digit()
            || (ch == '.'
                && chars
                    .clone()
                    .nth(1)
                    .is_some_and(|(_, next)| next.is_ascii_digit()))
        {
            tokens.push(scan_number(input, &mut chars));
            continue;
        }

        if ch == '\'' || ch == '"' {
            tokens.push(scan_string(&mut chars, ch));
            continue;
        }

        if is_identifier_start(ch) {
            let start = chars.peek().expect("peeked character exists").0;
            let mut end = start;

            while let Some((index, next)) = chars.peek().copied() {
                if is_identifier_continue(next) {
                    end = index + next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }

            let identifier = &input[start..end];
            tokens.push(match identifier {
                "let" => Token::Let,
                "var" => Token::Var,
                "const" => Token::Const,
                "if" => Token::If,
                "else" => Token::Else,
                "while" => Token::While,
                "function" => Token::Function,
                "return" => Token::Return,
                "true" => Token::True,
                "false" => Token::False,
                "null" => Token::Null,
                "undefined" => Token::Undefined,
                _ => Token::Identifier(identifier.to_owned()),
            });
            continue;
        }

        chars.next();
        tokens.push(match ch {
            '+' => Token::Plus,
            '-' => Token::Subtract,
            '*' => Token::Multiply,
            '/' => Token::Divide,
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
            '<' if take_if(&mut chars, '=') => Token::LessEqual,
            '<' => Token::LessThan,
            '>' if take_if(&mut chars, '=') => Token::GreaterEqual,
            '>' => Token::GreaterThan,
            '&' if take_if(&mut chars, '&') => Token::AndAnd,
            '|' if take_if(&mut chars, '|') => Token::OrOr,
            ';' => Token::Semicolon,
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            ',' => Token::Comma,
            other => Token::Unknown(other),
        });
    }

    tokens
}

fn scan_number(input: &str, chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> Token {
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

fn scan_string(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, quote: char) -> Token {
    chars.next(); // opening quote
    let mut value = String::new();
    let mut invalid_escape = false;

    while let Some((_, ch)) = chars.next() {
        if ch == quote {
            return if invalid_escape {
                Token::Invalid("string contains an unsupported escape".into())
            } else {
                Token::String(value)
            };
        }

        if ch == '\n' || ch == '\r' {
            return Token::Invalid("string literal contains a line break".into());
        }

        if ch == '\\' {
            match chars.next() {
                Some((_, 'n')) => value.push('\n'),
                Some((_, 'r')) => value.push('\r'),
                Some((_, 't')) => value.push('\t'),
                Some((_, '\\')) => value.push('\\'),
                Some((_, '\'')) => value.push('\''),
                Some((_, '"')) => value.push('"'),
                Some((_, '\n' | '\r')) => {
                    return Token::Invalid("string literal contains a line break".into());
                }
                Some((_, _)) => invalid_escape = true,
                None => return Token::Invalid("unterminated string literal".into()),
            }
        } else {
            value.push(ch);
        }
    }

    Token::Invalid("unterminated string literal".into())
}

fn take_if(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, expected: char) -> bool {
    if chars.peek().is_some_and(|(_, ch)| *ch == expected) {
        chars.next();
        true
    } else {
        false
    }
}

fn is_identifier_start(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphabetic()
}

fn is_identifier_continue(ch: char) -> bool {
    is_identifier_start(ch) || ch.is_ascii_digit()
}
