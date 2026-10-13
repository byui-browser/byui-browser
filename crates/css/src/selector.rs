//! CSS selector syntax parser.
//!
//! This module parses selector text into an AST. It does not yet match selectors
//! against the HTML DOM; matching is a separate stage of the CSS engine.

use std::fmt;

/// A comma-separated selector list, such as `h1, .title, #main`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectorList {
    pub selectors: Vec<ComplexSelector>,
}

/// A selector made from compounds connected by combinators.
/// `div.card > p.note` has three simple-selector groups? No: two compounds,
/// `div.card` and `p.note`, connected by `Child`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComplexSelector {
    /// A leading combinator is allowed only for relative selectors, such as
    /// the arguments of `:has(> img)`.
    pub leading_combinator: Option<Combinator>,
    pub compounds: Vec<CompoundSelector>,
    /// There is one combinator between each pair of compounds.
    pub combinators: Vec<Combinator>,
}

/// A collection of simple selectors that must all match the same element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    pub type_selector: Option<TypeSelector>,
    pub simples: Vec<SimpleSelector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSelector {
    /// Optional namespace prefix (`svg|rect`, `*|a`, or `|a`).
    pub namespace: Option<String>,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
    /// CSS Selectors Level 4 column combinator (`||`).
    Column,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimpleSelector {
    Id(String),
    Class(String),
    Attribute(AttributeSelector),
    Pseudo(PseudoSelector),
    /// Nesting selector (`&`) from CSS Nesting.
    Nesting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSelector {
    pub namespace: Option<String>,
    pub name: String,
    pub operator: Option<AttributeOperator>,
    pub value: Option<String>,
    /// Optional case-sensitivity flag (`i` or `s`).
    pub modifier: Option<char>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeOperator {
    Equals,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PseudoSelector {
    pub element: bool,
    pub name: String,
    /// Raw argument text inside parentheses, e.g. `2n+1` or `.item, #main`.
    pub argument: Option<String>,
    /// Parsed selector argument for selector-taking functions such as
    /// `:is()`, `:where()`, `:not()`, `:has()`, `:host()`, and `::slotted()`.
    /// Other function arguments (for example `:nth-child(2n + 1)`) remain raw.
    pub nested_selectors: Option<Box<SelectorList>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorParseError {
    pub position: usize,
    pub message: String,
}

impl fmt::Display for SelectorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at character {}", self.message, self.position)
    }
}

impl std::error::Error for SelectorParseError {}

/// Parse a full comma-separated CSS selector list.
///
/// Parses the major CSS Selectors Level 4 forms, including type/universal and
/// namespace selectors, IDs/classes/attributes, all common combinators,
/// pseudo-classes/elements, selector-list functions, relative `:has()` arguments,
/// the nesting selector, CSS comments, and escaped identifiers. Selector
/// matching and semantic validation (such as whether a pseudo-class is known)
/// are intentionally separate from parsing.
pub fn parse_selector_list(input: &str) -> Result<SelectorList, SelectorParseError> {
    let without_comments = remove_selector_comments(input);
    parse_selector_list_mode(&without_comments, false)
}

/// Removes CSS comments without touching comment-like text inside quoted strings.
fn remove_selector_comments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut quote = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                i += 1;
                out.push(chars[i]);
            } else if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            out.push(c);
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') { i += 1; }
            if i + 1 >= chars.len() { break; }
            i += 2;
            // A comment behaves like whitespace between selector tokens.
            out.push(' ');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn parse_selector_list_mode(input: &str, allow_relative: bool) -> Result<SelectorList, SelectorParseError> {
    let cleaned;
    let input = if input.contains("/*") {
        cleaned = remove_selector_comments(input);
        cleaned.as_str()
    } else { input };
    let mut parser = Parser::new(input);
    parser.skip_whitespace();
    if parser.is_eof() {
        return parser.error("selector cannot be empty");
    }

    let mut selectors = Vec::new();
    loop {
        selectors.push(parser.parse_complex_selector(allow_relative)?);
        parser.skip_whitespace();
        if parser.is_eof() {
            break;
        }
        if !parser.consume(',') {
            return parser.error("expected ',' between selectors");
        }
        parser.skip_whitespace();
        if parser.is_eof() || parser.peek() == Some(',') {
            return parser.error("expected selector after ','");
        }
    }
    Ok(SelectorList { selectors })
}

fn nth_of_selector_argument(argument: &str) -> Option<&str> {
    // The `of` keyword is an identifier token separated from the An+B formula
    // and selector list. CSS keywords are ASCII case-insensitive.
    let bytes = argument.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index].is_ascii_whitespace()
            && bytes[index + 1].eq_ignore_ascii_case(&b'o')
            && bytes[index + 2].eq_ignore_ascii_case(&b'f')
            && index + 3 < bytes.len()
            && bytes[index + 3].is_ascii_whitespace()
        {
            let selectors = argument[index + 3..].trim();
            return if selectors.is_empty() { None } else { Some(selectors) };
        }
        index += 1;
    }
    None
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(input: &str) -> Self {
        Self { chars: input.chars().collect(), pos: 0 }
    }

    fn peek(&self) -> Option<char> { self.chars.get(self.pos).copied() }
    fn peek_next(&self) -> Option<char> { self.chars.get(self.pos + 1).copied() }
    fn is_eof(&self) -> bool { self.pos >= self.chars.len() }
    fn consume(&mut self, c: char) -> bool {
        if self.peek() == Some(c) { self.pos += 1; true } else { false }
    }
    fn error<T>(&self, message: &str) -> Result<T, SelectorParseError> {
        Err(SelectorParseError { position: self.pos, message: message.to_owned() })
    }
    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        while self.peek().is_some_and(char::is_whitespace) { self.pos += 1; }
        self.pos != start
    }

    fn parse_complex_selector(&mut self, allow_relative: bool) -> Result<ComplexSelector, SelectorParseError> {
        let leading_combinator = if allow_relative {
            match (self.peek(), self.peek_next()) {
                (Some('>'), _) => { self.pos += 1; Some(Combinator::Child) }
                (Some('+'), _) => { self.pos += 1; Some(Combinator::NextSibling) }
                (Some('~'), _) => { self.pos += 1; Some(Combinator::SubsequentSibling) }
                (Some('|'), Some('|')) => { self.pos += 2; Some(Combinator::Column) }
                _ => None,
            }
        } else { None };
        self.skip_whitespace();
        let mut compounds = vec![self.parse_compound_selector()?];
        let mut combinators = Vec::new();
        loop {
            let had_space = self.skip_whitespace();
            if self.is_eof() || self.peek() == Some(',') { break; }

            let explicit = match (self.peek(), self.peek_next()) {
                (Some('>'), _) => Some((Combinator::Child, 1)),
                (Some('+'), _) => Some((Combinator::NextSibling, 1)),
                (Some('~'), _) => Some((Combinator::SubsequentSibling, 1)),
                (Some('|'), Some('|')) => Some((Combinator::Column, 2)),
                _ => None,
            };
            let combinator = if let Some((combinator, width)) = explicit {
                self.pos += width;
                self.skip_whitespace();
                combinator
            } else if had_space {
                Combinator::Descendant
            } else {
                return self.error("expected a combinator or end of selector");
            };

            if self.is_eof() || matches!(self.peek(), Some(',' | '>' | '+' | '~')) {
                return self.error("expected selector after combinator");
            }
            combinators.push(combinator);
            compounds.push(self.parse_compound_selector()?);
        }
        Ok(ComplexSelector { leading_combinator, compounds, combinators })
    }

    fn parse_compound_selector(&mut self) -> Result<CompoundSelector, SelectorParseError> {
        let mut compound = CompoundSelector::default();
        if self.starts_type_selector() {
            compound.type_selector = Some(self.parse_type_selector()?);
        }

        loop {
            match self.peek() {
                Some('#') => {
                    self.pos += 1;
                    compound.simples.push(SimpleSelector::Id(self.parse_identifier("ID")?));
                }
                Some('.') => {
                    self.pos += 1;
                    compound.simples.push(SimpleSelector::Class(self.parse_identifier("class")?));
                }
                Some('[') => compound.simples.push(SimpleSelector::Attribute(self.parse_attribute_selector()?)),
                Some(':') => compound.simples.push(SimpleSelector::Pseudo(self.parse_pseudo_selector()?)),
                Some('&') => { self.pos += 1; compound.simples.push(SimpleSelector::Nesting); },
                _ => break,
            }
        }

        if compound.type_selector.is_none() && compound.simples.is_empty() {
            return self.error("expected a type selector, class, ID, attribute, or pseudo-selector");
        }
        Ok(compound)
    }

    fn starts_type_selector(&self) -> bool {
        match self.peek() {
            Some('*') => true,
            Some('|') => true,
            Some('\\') => true,
            Some(c) => is_name_start(c),
            None => false,
        }
    }

    fn parse_type_selector(&mut self) -> Result<TypeSelector, SelectorParseError> {
        let (namespace, name) = if self.consume('|') {
            (Some(String::new()), self.parse_name_or_star("type selector")?)
        } else {
            let first = self.parse_name_or_star("type selector")?;
            if self.consume('|') {
                (Some(first), self.parse_name_or_star("type selector after namespace")?)
            } else {
                (None, first)
            }
        };
        Ok(TypeSelector { namespace, name })
    }

    fn parse_name_or_star(&mut self, context: &str) -> Result<String, SelectorParseError> {
        if self.consume('*') { return Ok("*".to_owned()); }
        self.parse_identifier(context)
    }

    fn parse_identifier(&mut self, context: &str) -> Result<String, SelectorParseError> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if is_name_char(c) {
                self.pos += 1;
            } else if c == '\\' {
                self.pos += 1;
                let Some(escaped) = self.peek() else { return self.error("incomplete CSS escape"); };
                if escaped == '\n' || escaped == '\r' || escaped == '\u{000C}' {
                    return self.error("CSS escapes cannot escape a newline");
                }
                if escaped.is_ascii_hexdigit() {
                    let mut digits = 0;
                    while self.peek().is_some_and(|next| next.is_ascii_hexdigit()) && digits < 6 {
                        self.pos += 1;
                        digits += 1;
                    }
                    if self.peek().is_some_and(char::is_whitespace) { self.pos += 1; }
                } else {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        if self.pos == start { return self.error(&format!("expected {context} name")); }
        Ok(self.chars[start..self.pos].iter().collect())
    }

    fn parse_attribute_selector(&mut self) -> Result<AttributeSelector, SelectorParseError> {
        self.pos += 1; // '['
        self.skip_whitespace();
        let (namespace, name) = self.parse_attribute_name()?;
        self.skip_whitespace();
        if self.consume(']') {
            return Ok(AttributeSelector { namespace, name, operator: None, value: None, modifier: None });
        }

        let operator = if self.consume('=') { AttributeOperator::Equals }
        else if self.consume('~') && self.consume('=') { AttributeOperator::Includes }
        else if self.consume('|') && self.consume('=') { AttributeOperator::DashMatch }
        else if self.consume('^') && self.consume('=') { AttributeOperator::Prefix }
        else if self.consume('$') && self.consume('=') { AttributeOperator::Suffix }
        else if self.consume('*') && self.consume('=') { AttributeOperator::Substring }
        else { return self.error("expected attribute operator or ']' "); };

        self.skip_whitespace();
        let value = self.parse_attribute_value()?;
        self.skip_whitespace();
        let modifier = match self.peek() {
            Some(c @ ('i' | 'I' | 's' | 'S')) => { self.pos += 1; Some(c.to_ascii_lowercase()) }
            _ => None,
        };
        self.skip_whitespace();
        if !self.consume(']') { return self.error("expected ']' to close attribute selector"); }
        Ok(AttributeSelector { namespace, name, operator: Some(operator), value: Some(value), modifier })
    }

    fn parse_attribute_name(&mut self) -> Result<(Option<String>, String), SelectorParseError> {
        if self.consume('|') {
            return Ok((Some(String::new()), self.parse_identifier("attribute")?));
        }
        if self.consume('*') {
            if !self.consume('|') {
                return self.error("expected '|' after wildcard attribute namespace");
            }
            return Ok((Some("*".to_owned()), self.parse_identifier("attribute after namespace")?));
        }
        let first = self.parse_identifier("attribute")?;
        if self.consume('|') {
            Ok((Some(first), self.parse_identifier("attribute after namespace")?))
        } else {
            Ok((None, first))
        }
    }

    fn parse_attribute_value(&mut self) -> Result<String, SelectorParseError> {
        if let Some(quote @ ('\'' | '"')) = self.peek() {
            self.pos += 1;
            let start = self.pos;
            while !self.is_eof() && self.peek() != Some(quote) {
                if self.consume('\\') && !self.is_eof() { self.pos += 1; } else if !self.is_eof() { self.pos += 1; }
            }
            if self.is_eof() { return self.error("unterminated quoted attribute value"); }
            let value: String = self.chars[start..self.pos].iter().collect();
            self.pos += 1;
            Ok(value)
        } else {
            self.parse_identifier("attribute value")
        }
    }

    fn parse_pseudo_selector(&mut self) -> Result<PseudoSelector, SelectorParseError> {
        self.pos += 1; // ':'
        let element = self.consume(':');
        let name = self.parse_identifier("pseudo-selector")?;
        let argument = if self.consume('(') { Some(self.parse_function_argument()?) } else { None };
        let lower_name = name.to_ascii_lowercase();
        let nested_text = match (lower_name.as_str(), argument.as_deref()) {
            ("is" | "where" | "not" | "has" | "host" | "host-context" | "slotted", Some(text)) => Some(text),
            ("nth-child" | "nth-last-child", Some(text)) => nth_of_selector_argument(text),
            _ => None,
        };
        let nested_selectors = if let Some(text) = nested_text {
            Some(Box::new(parse_selector_list_mode(text, lower_name == "has")?))
        } else { None };
        Ok(PseudoSelector { element, name, argument, nested_selectors })
    }

    fn parse_function_argument(&mut self) -> Result<String, SelectorParseError> {
        let start = self.pos;
        let mut depth = 1usize;
        let mut quote: Option<char> = None;
        while let Some(c) = self.peek() {
            if let Some(q) = quote {
                if c == '\\' { self.pos += 1; if !self.is_eof() { self.pos += 1; } continue; }
                if c == q { quote = None; }
                self.pos += 1;
                continue;
            }
            match c {
                '\'' | '"' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let argument: String = self.chars[start..self.pos].iter().collect();
                        self.pos += 1;
                        return Ok(argument.trim().to_owned());
                    }
                }
                '\\' => { self.pos += 1; if !self.is_eof() { self.pos += 1; } continue; }
                _ => {}
            }
            self.pos += 1;
        }
        self.error("unterminated pseudo-selector argument")
    }
}

fn is_name_start(c: char) -> bool { c == '_' || c == '-' || c.is_alphabetic() || c as u32 >= 0x80 }
fn is_name_char(c: char) -> bool { is_name_start(c) || c.is_ascii_digit() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_selector_list() {
        let parsed = parse_selector_list("h1, .title, #main").unwrap();
        assert_eq!(parsed.selectors.len(), 3);
    }

    #[test]
    fn parses_compound_selector() {
        let parsed = parse_selector_list("div.card#main").unwrap();
        let compound = &parsed.selectors[0].compounds[0];
        assert_eq!(compound.type_selector.as_ref().unwrap().name, "div");
        assert!(compound.simples.contains(&SimpleSelector::Class("card".into())));
        assert!(compound.simples.contains(&SimpleSelector::Id("main".into())));
    }

    #[test]
    fn parses_all_four_combinators() {
        for (input, expected) in [
            ("div p", Combinator::Descendant),
            ("div > p", Combinator::Child),
            ("h1 + p", Combinator::NextSibling),
            ("h1 ~ p", Combinator::SubsequentSibling),
        ] {
            assert_eq!(parse_selector_list(input).unwrap().selectors[0].combinators, vec![expected]);
        }
    }

    #[test]
    fn parses_attribute_selector() {
        let parsed = parse_selector_list("input[type=\"text\" i]").unwrap();
        let SimpleSelector::Attribute(attribute) = &parsed.selectors[0].compounds[0].simples[0] else { panic!("expected attribute") };
        assert_eq!(attribute.name, "type");
        assert_eq!(attribute.value.as_deref(), Some("text"));
        assert_eq!(attribute.modifier, Some('i'));
    }

    #[test]
    fn parses_wildcard_attribute_namespace() {
        let parsed = parse_selector_list("[*|href]").unwrap();
        let SimpleSelector::Attribute(attribute) = &parsed.selectors[0].compounds[0].simples[0] else { panic!("expected attribute") };
        assert_eq!(attribute.namespace.as_deref(), Some("*"));
        assert_eq!(attribute.name, "href");
    }

    #[test]
    fn parses_pseudo_selector_with_nested_parentheses() {
        let parsed = parse_selector_list("li:nth-child(2n + 1), a:not(.active)").unwrap();
        assert_eq!(parsed.selectors.len(), 2);
        let SimpleSelector::Pseudo(pseudo) = &parsed.selectors[0].compounds[0].simples[0] else { panic!("expected pseudo") };
        assert_eq!(pseudo.name, "nth-child");
        assert_eq!(pseudo.argument.as_deref(), Some("2n + 1"));
    }

    #[test]
    fn parses_nested_selector_functions_and_relative_has_selectors() {
        let parsed = parse_selector_list("article:has(> img.hero), :is(h1, h2, .title)").unwrap();
        assert_eq!(parsed.selectors.len(), 2);
        let SimpleSelector::Pseudo(has) = &parsed.selectors[0].compounds[0].simples[0] else { panic!("expected :has") };
        let nested = has.nested_selectors.as_ref().unwrap();
        assert_eq!(nested.selectors[0].leading_combinator, Some(Combinator::Child));
        let SimpleSelector::Pseudo(is) = &parsed.selectors[1].compounds[0].simples[0] else { panic!("expected :is") };
        assert_eq!(is.nested_selectors.as_ref().unwrap().selectors.len(), 3);
    }

    #[test]
    fn parses_nth_child_of_selector_list() {
        let parsed = parse_selector_list("li:nth-child(2n + 1 of .item, #featured)").unwrap();
        let SimpleSelector::Pseudo(pseudo) = &parsed.selectors[0].compounds[0].simples[0] else { panic!("expected pseudo") };
        assert_eq!(pseudo.nested_selectors.as_ref().unwrap().selectors.len(), 2);
    }

    #[test]
    fn parses_column_combinator_and_nesting_selector() {
        let parsed = parse_selector_list("col.selected || td, & > .child").unwrap();
        assert_eq!(parsed.selectors[0].combinators, vec![Combinator::Column]);
        assert!(parsed.selectors[1].compounds[0].simples.contains(&SimpleSelector::Nesting));
    }

    #[test]
    fn ignores_comments_between_selector_tokens() {
        let parsed = parse_selector_list("div/* comment */.card, [title=\"a/*b\" i]").unwrap();
        assert_eq!(parsed.selectors.len(), 2);
    }

    #[test]
    fn parses_hex_escaped_identifiers() {
        assert!(parse_selector_list(r".\31 23item").is_ok());
    }

    #[test]
    fn rejects_invalid_selector_syntax() {
        assert!(parse_selector_list("").is_err());
        assert!(parse_selector_list("div >").is_err());
        assert!(parse_selector_list("div,").is_err());
        assert!(parse_selector_list("[title=").is_err());
    }
}
