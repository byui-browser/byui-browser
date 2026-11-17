//! A forgiving JavaScript tokenizer used only to *detect* which language
//! features and APIs a page's scripts use.
//!
//! It deliberately does not reuse the engine's lexer: the point of the audit
//! is to describe source the engine cannot handle, so the scanner must accept
//! all of modern JavaScript (template literals, regular expressions, private
//! names, numeric separators, ...) and never fail. It is not a parser; the
//! detectors in [`super::script`] look at short token windows.

/// One significant token. Comments and whitespace are dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    /// An identifier or keyword, such as `for` or `document`.
    Ident(String),
    /// A punctuator such as `?.`, `=>`, or `{`.
    Punct(&'static str),
    /// A string literal. The flag is set when it contains an escape sequence
    /// other than a simple quote or backslash escape.
    Str { escapes: bool },
    /// A template literal (or its first chunk when it has substitutions).
    Template,
    /// A numeric literal's source text.
    Num(String),
    /// A regular expression literal.
    Regex,
    /// A private name such as `#count`.
    Private,
}

/// A token and the 1-based line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Token {
    pub tok: Tok,
    pub line: usize,
}

impl Token {
    /// Whether this token is the identifier or keyword `name`.
    pub fn is_ident(&self, name: &str) -> bool {
        matches!(&self.tok, Tok::Ident(ident) if ident == name)
    }

    /// Whether this token is the punctuator `punct`.
    pub fn is_punct(&self, punct: &str) -> bool {
        matches!(self.tok, Tok::Punct(p) if p == punct)
    }

    /// The identifier text, if this is an identifier.
    pub fn ident(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(ident) => Some(ident),
            _ => None,
        }
    }
}

/// Punctuators, longest first so the scanner takes the longest match.
const PUNCTUATORS: &[&str] = &[
    ">>>=", "...", "===", "!==", "**=", "<<=", ">>=", ">>>", "&&=", "||=", "??=", "=>", "==", "!=",
    "<=", ">=", "&&", "||", "??", "?.", "++", "--", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=",
    "**", "<<", ">>", "{", "}", "(", ")", "[", "]", ";", ",", "<", ">", "+", "-", "*", "/", "%",
    "&", "|", "^", "!", "~", "?", ":", "=", ".", "@",
];

/// Keywords after which a `/` starts a regular expression, not a division.
const REGEX_PREFIX_KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "instanceof",
    "yield",
    "await",
];

/// Splits `source` into tokens. Never fails; unknown characters are skipped.
pub(crate) fn tokenize(source: &str) -> Vec<Token> {
    Scanner {
        chars: source.chars().collect(),
        pos: 0,
        line: 1,
        tokens: Vec::new(),
        template_braces: Vec::new(),
        brace_depth: 0,
    }
    .run()
}

struct Scanner {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    tokens: Vec<Token>,
    /// Brace depths at which an open `${` substitution resumes its template.
    template_braces: Vec<usize>,
    brace_depth: usize,
}

impl Scanner {
    fn peek(&self, offset: usize) -> Option<char> {
        self.chars.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.get(self.pos).copied()?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn push(&mut self, tok: Tok, line: usize) {
        self.tokens.push(Token { tok, line });
    }

    fn run(mut self) -> Vec<Token> {
        while let Some(c) = self.peek(0) {
            let line = self.line;
            if c.is_whitespace() {
                self.bump();
            } else if c == '/' && self.peek(1) == Some('/') {
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.bump();
                }
            } else if c == '/' && self.peek(1) == Some('*') {
                self.pos += 2;
                while self.peek(0).is_some()
                    && !(self.peek(0) == Some('*') && self.peek(1) == Some('/'))
                {
                    self.bump();
                }
                self.pos = (self.pos + 2).min(self.chars.len());
            } else if c == '\'' || c == '"' {
                let escapes = self.string(c);
                self.push(Tok::Str { escapes }, line);
            } else if c == '`' {
                self.bump();
                self.push(Tok::Template, line);
                self.template_body();
            } else if c == '}' && self.template_braces.last() == Some(&self.brace_depth) {
                // The end of a `${...}` substitution: continue the template.
                self.bump();
                self.template_braces.pop();
                self.template_body();
            } else if c.is_ascii_digit()
                || (c == '.' && self.peek(1).is_some_and(|d| d.is_ascii_digit()))
            {
                let start = self.pos;
                self.number();
                let text: String = self.chars[start..self.pos].iter().collect();
                self.push(Tok::Num(text), line);
            } else if c == '#' && self.peek(1).is_some_and(is_ident_start) {
                self.bump();
                self.identifier();
                self.push(Tok::Private, line);
            } else if is_ident_start(c) {
                let ident = self.identifier();
                self.push(Tok::Ident(ident), line);
            } else if c == '/' && self.regex_allowed() {
                self.regex();
                self.push(Tok::Regex, line);
            } else if let Some(punct) = self.punctuator() {
                match punct {
                    "{" => self.brace_depth += 1,
                    "}" => self.brace_depth = self.brace_depth.saturating_sub(1),
                    _ => {}
                }
                self.push(Tok::Punct(punct), line);
            } else {
                self.bump();
            }
        }
        self.tokens
    }

    fn identifier(&mut self) -> String {
        let start = self.pos;
        while self
            .peek(0)
            .is_some_and(|c| is_ident_start(c) || c.is_ascii_digit())
        {
            self.bump();
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// Consumes a numeric literal: a radix-prefixed integer, or a decimal
    /// with optional fraction and exponent; either may end in a BigInt `n`.
    fn number(&mut self) {
        let digits = |scanner: &mut Self| {
            while scanner
                .peek(0)
                .is_some_and(|c| c.is_ascii_digit() || c == '_')
            {
                scanner.bump();
            }
        };
        if self.peek(0) == Some('0')
            && matches!(self.peek(1), Some('x' | 'X' | 'b' | 'B' | 'o' | 'O'))
        {
            self.pos += 2;
            while self
                .peek(0)
                .is_some_and(|c| c.is_ascii_hexdigit() || c == '_')
            {
                self.bump();
            }
        } else {
            digits(self);
            if self.peek(0) == Some('.') && self.peek(1).is_none_or(|c| !is_ident_start(c)) {
                self.bump();
                digits(self);
            }
            if matches!(self.peek(0), Some('e' | 'E'))
                && (self.peek(1).is_some_and(|c| c.is_ascii_digit())
                    || (matches!(self.peek(1), Some('+' | '-'))
                        && self.peek(2).is_some_and(|c| c.is_ascii_digit())))
            {
                self.pos += 2;
                digits(self);
            }
        }
        if self.peek(0) == Some('n') {
            self.bump();
        }
    }

    /// Consumes a quoted string; returns whether it held a notable escape.
    fn string(&mut self, quote: char) -> bool {
        self.bump();
        let mut escapes = false;
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    if let Some(escaped) = self.bump() {
                        escapes |= !matches!(escaped, '\'' | '"' | '\\' | '\n');
                    }
                }
                '\n' => break,
                c if c == quote => break,
                _ => {}
            }
        }
        escapes
    }

    /// Consumes template text up to the closing backtick or a `${`.
    fn template_body(&mut self) {
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '`' => return,
                '$' if self.peek(0) == Some('{') => {
                    self.bump();
                    self.template_braces.push(self.brace_depth);
                    return;
                }
                _ => {}
            }
        }
    }

    fn regex(&mut self) {
        self.bump();
        let mut in_class = false;
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => break,
                '\n' => return,
                _ => {}
            }
        }
        while self.peek(0).is_some_and(|c| c.is_ascii_alphabetic()) {
            self.bump();
        }
    }

    /// A `/` begins a regular expression unless it follows something that
    /// ends an expression (a value, `)`, `]`, or `}`).
    fn regex_allowed(&self) -> bool {
        match self.tokens.last().map(|token| &token.tok) {
            None => true,
            Some(Tok::Punct(p)) => !matches!(*p, ")" | "]" | "}" | "++" | "--"),
            Some(Tok::Ident(ident)) => REGEX_PREFIX_KEYWORDS.contains(&ident.as_str()),
            Some(_) => false,
        }
    }

    fn punctuator(&mut self) -> Option<&'static str> {
        let rest = &self.chars[self.pos..];
        let found = PUNCTUATORS.iter().copied().find(|punct| {
            punct.chars().count() <= rest.len() && punct.chars().zip(rest).all(|(a, b)| a == *b)
        })?;
        // `a?.5:b` is a conditional with a decimal, not optional chaining.
        let found = if found == "?." && self.peek(2).is_some_and(|c| c.is_ascii_digit()) {
            "?"
        } else {
            found
        };
        self.pos += found.len();
        Some(found)
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(source: &str) -> Vec<Tok> {
        tokenize(source)
            .into_iter()
            .map(|token| token.tok)
            .collect()
    }

    #[test]
    fn splits_identifiers_punctuators_and_literals() {
        assert_eq!(
            toks("a?.b ?? c >>>= 1_000n;"),
            [
                Tok::Ident("a".into()),
                Tok::Punct("?."),
                Tok::Ident("b".into()),
                Tok::Punct("??"),
                Tok::Ident("c".into()),
                Tok::Punct(">>>="),
                Tok::Num("1_000n".into()),
                Tok::Punct(";"),
            ]
        );
        assert_eq!(
            toks("x ? .5 : 1e-3"),
            [
                Tok::Ident("x".into()),
                Tok::Punct("?"),
                Tok::Num(".5".into()),
                Tok::Punct(":"),
                Tok::Num("1e-3".into()),
            ]
        );
    }

    #[test]
    fn distinguishes_regex_from_division_and_skips_comments() {
        assert_eq!(
            toks("a / b; return /x[/]y/g.test(s) // done\n/* c */"),
            [
                Tok::Ident("a".into()),
                Tok::Punct("/"),
                Tok::Ident("b".into()),
                Tok::Punct(";"),
                Tok::Ident("return".into()),
                Tok::Regex,
                Tok::Punct("."),
                Tok::Ident("test".into()),
                Tok::Punct("("),
                Tok::Ident("s".into()),
                Tok::Punct(")"),
            ]
        );
    }

    #[test]
    fn templates_resume_after_substitutions_and_lines_are_counted() {
        let tokens = tokenize("`a ${ {x: 1}.x } b`;\nnext");
        let kinds: Vec<_> = tokens.iter().map(|token| token.tok.clone()).collect();
        assert_eq!(
            kinds,
            [
                Tok::Template,
                Tok::Punct("{"),
                Tok::Ident("x".into()),
                Tok::Punct(":"),
                Tok::Num("1".into()),
                Tok::Punct("}"),
                Tok::Punct("."),
                Tok::Ident("x".into()),
                Tok::Punct(";"),
                Tok::Ident("next".into()),
            ]
        );
        assert_eq!(tokens.last().unwrap().line, 2);
        assert_eq!(
            toks("'\\x41' \"plain\" #priv"),
            [
                Tok::Str { escapes: true },
                Tok::Str { escapes: false },
                Tok::Private
            ]
        );
    }
}
