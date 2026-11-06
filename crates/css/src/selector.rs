//! CSS selector syntax: the selector AST and a token-based parser.
//!
//! [`parse_selector_list`] tokenizes selector text with
//! [`crate::tokenizer::tokenize`] and parses the tokens into a
//! [`SelectorList`]. The stylesheet parser reuses the same token parser for
//! rule preludes. Matching selectors against the DOM lives in the cascade's
//! matcher, not here.

use crate::parser::skip_component_value;
use crate::tokenizer::{Token, TokenKind, source_text, tokenize, trim_whitespace};
use std::fmt;

/// A comma-separated selector list, such as `h1, .title, #main`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectorList {
    /// The complex selectors of the list, in source order.
    pub selectors: Vec<ComplexSelector>,
}

/// A selector made from compounds connected by combinators.
///
/// `div.card > p.note` has two compounds, `div.card` and `p.note`, connected
/// by [`Combinator::Child`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ComplexSelector {
    /// A leading combinator is allowed only for relative selectors, such as
    /// the arguments of `:has(> img)`.
    pub leading_combinator: Option<Combinator>,
    /// Compounds from left to right. A parsed selector has at least one.
    pub compounds: Vec<CompoundSelector>,
    /// There is one combinator between each pair of compounds:
    /// `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
    pub combinators: Vec<Combinator>,
}

/// A collection of simple selectors that must all match the same element.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompoundSelector {
    /// The type or universal selector (`div`, `*`), if the compound has one.
    pub type_selector: Option<TypeSelector>,
    /// The remaining simple selectors, in source order.
    pub simples: Vec<SimpleSelector>,
}

/// A type (`div`) or universal (`*`) selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSelector {
    /// Optional namespace prefix (`svg|rect`, `*|a`, or `|a`). An empty string
    /// means "no namespace" and `*` means any namespace.
    pub namespace: Option<String>,
    /// Element name as written, or `*` for the universal selector.
    pub name: String,
}

/// How two compounds of a complex selector relate in the document tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    /// Whitespace: the left compound matches an ancestor.
    Descendant,
    /// `>`: the left compound matches the parent.
    Child,
    /// `+`: the left compound matches the immediately preceding sibling.
    NextSibling,
    /// `~`: the left compound matches any preceding sibling.
    SubsequentSibling,
    /// CSS Selectors Level 4 column combinator (`||`).
    Column,
}

/// One simple selector inside a compound, other than the type selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimpleSelector {
    /// `#name`. The value is unescaped.
    Id(String),
    /// `.name`. The value is unescaped.
    Class(String),
    /// `[name]`, `[name=value]`, and the other attribute forms.
    Attribute(AttributeSelector),
    /// A pseudo-class (`:hover`) or pseudo-element (`::before`).
    Pseudo(PseudoSelector),
    /// Nesting selector (`&`) from CSS Nesting.
    Nesting,
}

/// An attribute selector such as `[type="text" i]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeSelector {
    /// Optional namespace prefix, as for [`TypeSelector::namespace`].
    pub namespace: Option<String>,
    /// Attribute name as written.
    pub name: String,
    /// Comparison operator, or `None` for a presence test (`[name]`).
    pub operator: Option<AttributeOperator>,
    /// Unescaped comparison value; present exactly when `operator` is.
    pub value: Option<String>,
    /// Optional case-sensitivity flag, lowercased (`i` or `s`).
    pub modifier: Option<char>,
}

/// Attribute selector comparison operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeOperator {
    /// `=`: the value equals the given value.
    Equals,
    /// `~=`: the whitespace-separated value list contains the given word.
    Includes,
    /// `|=`: the value equals the given value or starts with it plus `-`.
    DashMatch,
    /// `^=`: the value starts with the given value.
    Prefix,
    /// `$=`: the value ends with the given value.
    Suffix,
    /// `*=`: the value contains the given value.
    Substring,
}

/// A pseudo-class or pseudo-element, with its optional argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PseudoSelector {
    /// `true` for pseudo-elements written with `::`.
    pub element: bool,
    /// Name as written, without colons or parenthesis.
    pub name: String,
    /// Raw argument text inside parentheses, e.g. `2n+1` or `.item, #main`,
    /// with comments removed.
    pub argument: Option<String>,
    /// Parsed selector argument for selector-taking functions such as
    /// `:is()`, `:where()`, `:not()`, `:has()`, `:host()`, and `::slotted()`,
    /// and for the `of S` part of `:nth-child()`. Other function arguments
    /// (for example `:nth-child(2n + 1)`) remain raw.
    pub nested_selectors: Option<Box<SelectorList>>,
}

/// Why selector text failed to parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorParseError {
    /// Byte offset into the parsed text where the problem was found.
    pub position: usize,
    /// Human-readable description of the problem.
    pub message: String,
}

impl fmt::Display for SelectorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.position)
    }
}

impl std::error::Error for SelectorParseError {}

/// Parse a full comma-separated CSS selector list.
///
/// The input is tokenized with [`crate::tokenizer::tokenize`] and parsed
/// from tokens. This covers the major CSS Selectors Level 4 forms, including
/// type/universal and namespace selectors, IDs/classes/attributes, all common
/// combinators, pseudo-classes/elements, selector-list functions, relative
/// `:has()` arguments, the nesting selector, comments, and escaped
/// identifiers. Identifier and string values in the AST are unescaped.
///
/// `:is()` and `:where()` take forgiving selector lists (invalid entries are
/// dropped); every other list is invalid if any entry is invalid. Semantic
/// validation, such as whether a pseudo-class is known, is left to matching.
pub fn parse_selector_list(input: &str) -> Result<SelectorList, SelectorParseError> {
    log::trace!("parsing selector list `{input}`");
    let tokens = tokenize(input);
    let result = parse_selector_tokens(input, &tokens);
    if let Err(error) = &result {
        log::debug!("invalid selector `{input}`: {error}");
    }
    result
}

/// Parses a selector list from tokens of `source`.
///
/// Error positions are byte offsets into `source`.
pub(crate) fn parse_selector_tokens(
    source: &str,
    tokens: &[Token<'_>],
) -> Result<SelectorList, SelectorParseError> {
    let end = tokens.last().map_or(0, |token| token.span.end);
    parse_list(source, tokens, end, false, false)
}

/// Parses a comma-separated list. `end_offset` is the error position used
/// when the tokens run out. A `forgiving` list drops invalid entries.
fn parse_list(
    source: &str,
    tokens: &[Token<'_>],
    end_offset: usize,
    allow_relative: bool,
    forgiving: bool,
) -> Result<SelectorList, SelectorParseError> {
    let mut selectors = Vec::new();
    let mut start = 0;
    let mut index = 0;
    loop {
        let at_end = index >= tokens.len();
        if !at_end && tokens[index].kind != TokenKind::Comma {
            index = skip_component_value(tokens, index);
            continue;
        }
        let piece = trim_whitespace(&tokens[start..index]);
        let piece_end = tokens
            .get(index)
            .map_or(end_offset, |token| token.span.start);
        if piece.is_empty() {
            if !forgiving {
                let message = if at_end && start == 0 {
                    "selector cannot be empty"
                } else {
                    "expected selector after ','"
                };
                return Err(SelectorParseError {
                    position: piece_end,
                    message: message.to_owned(),
                });
            }
        } else {
            let mut parser = Parser {
                source,
                tokens: piece,
                pos: 0,
                end_offset: piece_end,
            };
            match parser.parse_complex_selector(allow_relative) {
                Ok(selector) => selectors.push(selector),
                Err(error) if forgiving => {
                    log::debug!("dropping invalid entry from forgiving selector list: {error}");
                }
                Err(error) => return Err(error),
            }
        }
        if at_end {
            break;
        }
        index += 1;
        start = index;
    }
    Ok(SelectorList { selectors })
}

struct Parser<'s, 't, 'a> {
    source: &'s str,
    tokens: &'t [Token<'a>],
    pos: usize,
    /// Error position once the tokens run out.
    end_offset: usize,
}

impl<'s, 't, 'a> Parser<'s, 't, 'a> {
    fn peek(&self) -> Option<&TokenKind<'a>> {
        self.peek_at(0)
    }

    fn peek_at(&self, offset: usize) -> Option<&TokenKind<'a>> {
        self.tokens.get(self.pos + offset).map(|token| &token.kind)
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn offset(&self) -> usize {
        self.tokens
            .get(self.pos)
            .map_or(self.end_offset, |token| token.span.start)
    }

    fn error<T>(&self, message: &str) -> Result<T, SelectorParseError> {
        Err(SelectorParseError {
            position: self.offset(),
            message: message.to_owned(),
        })
    }

    fn skip_whitespace(&mut self) -> bool {
        let start = self.pos;
        while self.peek() == Some(&TokenKind::Whitespace) {
            self.pos += 1;
        }
        self.pos != start
    }

    /// Whether the next two tokens are directly adjacent `|` delimiters.
    fn at_column_combinator(&self) -> bool {
        self.peek() == Some(&TokenKind::Delim('|'))
            && self.peek_at(1) == Some(&TokenKind::Delim('|'))
            && self.tokens[self.pos].span.end == self.tokens[self.pos + 1].span.start
    }

    fn peek_combinator(&self) -> Option<(Combinator, usize)> {
        match self.peek()? {
            TokenKind::Delim('>') => Some((Combinator::Child, 1)),
            TokenKind::Delim('+') => Some((Combinator::NextSibling, 1)),
            TokenKind::Delim('~') => Some((Combinator::SubsequentSibling, 1)),
            TokenKind::Delim('|') if self.at_column_combinator() => Some((Combinator::Column, 2)),
            _ => None,
        }
    }

    fn parse_complex_selector(
        &mut self,
        allow_relative: bool,
    ) -> Result<ComplexSelector, SelectorParseError> {
        self.skip_whitespace();
        let mut leading_combinator = None;
        if allow_relative && let Some((combinator, width)) = self.peek_combinator() {
            self.pos += width;
            leading_combinator = Some(combinator);
            self.skip_whitespace();
        }
        let mut compounds = vec![self.parse_compound_selector()?];
        let mut combinators = Vec::new();
        loop {
            let had_space = self.skip_whitespace();
            if self.is_eof() {
                break;
            }
            let combinator = if let Some((combinator, width)) = self.peek_combinator() {
                self.pos += width;
                self.skip_whitespace();
                combinator
            } else if had_space {
                Combinator::Descendant
            } else {
                return self.error("expected a combinator or end of selector");
            };
            if self.is_eof() || self.peek_combinator().is_some() {
                return self.error("expected selector after combinator");
            }
            combinators.push(combinator);
            compounds.push(self.parse_compound_selector()?);
        }
        Ok(ComplexSelector {
            leading_combinator,
            compounds,
            combinators,
        })
    }

    fn parse_compound_selector(&mut self) -> Result<CompoundSelector, SelectorParseError> {
        let mut compound = CompoundSelector::default();
        if matches!(
            self.peek(),
            Some(TokenKind::Ident(_) | TokenKind::Delim('*' | '|'))
        ) && !self.at_column_combinator()
        {
            compound.type_selector = Some(self.parse_type_selector()?);
        }

        loop {
            match self.peek() {
                Some(TokenKind::Hash { value, is_id }) => {
                    if !is_id {
                        return self.error("an ID selector must be a valid identifier");
                    }
                    let id = value.to_string();
                    self.pos += 1;
                    compound.simples.push(SimpleSelector::Id(id));
                }
                Some(TokenKind::Delim('.')) => {
                    self.pos += 1;
                    let Some(TokenKind::Ident(name)) = self.peek() else {
                        return self.error("expected class name");
                    };
                    let class = name.to_string();
                    self.pos += 1;
                    compound.simples.push(SimpleSelector::Class(class));
                }
                Some(TokenKind::OpenSquare) => {
                    let attribute = self.parse_attribute_selector()?;
                    compound.simples.push(SimpleSelector::Attribute(attribute));
                }
                Some(TokenKind::Colon) => {
                    let pseudo = self.parse_pseudo_selector()?;
                    compound.simples.push(SimpleSelector::Pseudo(pseudo));
                }
                Some(TokenKind::Delim('&')) => {
                    self.pos += 1;
                    compound.simples.push(SimpleSelector::Nesting);
                }
                _ => break,
            }
        }

        if compound.type_selector.is_none() && compound.simples.is_empty() {
            return self
                .error("expected a type selector, class, ID, attribute, or pseudo-selector");
        }
        Ok(compound)
    }

    fn parse_type_selector(&mut self) -> Result<TypeSelector, SelectorParseError> {
        if self.peek() == Some(&TokenKind::Delim('|')) {
            self.pos += 1;
            let name = self.parse_name_or_star("type selector")?;
            return Ok(TypeSelector {
                namespace: Some(String::new()),
                name,
            });
        }
        let first = self.parse_name_or_star("type selector")?;
        let namespaced = self.peek() == Some(&TokenKind::Delim('|'))
            && matches!(
                self.peek_at(1),
                Some(TokenKind::Ident(_) | TokenKind::Delim('*'))
            );
        if namespaced {
            self.pos += 1;
            let name = self.parse_name_or_star("type selector after namespace")?;
            return Ok(TypeSelector {
                namespace: Some(first),
                name,
            });
        }
        Ok(TypeSelector {
            namespace: None,
            name: first,
        })
    }

    fn parse_name_or_star(&mut self, context: &str) -> Result<String, SelectorParseError> {
        let name = match self.peek() {
            Some(TokenKind::Delim('*')) => "*".to_owned(),
            Some(TokenKind::Ident(name)) => name.to_string(),
            _ => return self.error(&format!("expected {context} name")),
        };
        self.pos += 1;
        Ok(name)
    }

    fn parse_ident(&mut self, context: &str) -> Result<String, SelectorParseError> {
        let Some(TokenKind::Ident(name)) = self.peek() else {
            return self.error(&format!("expected {context} name"));
        };
        let name = name.to_string();
        self.pos += 1;
        Ok(name)
    }

    /// Returns the tokens inside the block or function at the cursor and the
    /// byte offset of its closing token, and moves past it. Reports `message`
    /// when it is not closed by `closer`.
    fn take_block(
        &mut self,
        closer: &TokenKind<'_>,
        message: &str,
    ) -> Result<(&'t [Token<'a>], usize), SelectorParseError> {
        let open = self.pos;
        let end = skip_component_value(self.tokens, open);
        if self
            .tokens
            .get(end.wrapping_sub(1))
            .map(|token| &token.kind)
            != Some(closer)
            || end == open + 1
        {
            self.pos = end;
            return self.error(message);
        }
        self.pos = end;
        let tokens: &'t [Token<'a>] = self.tokens;
        Ok((&tokens[open + 1..end - 1], tokens[end - 1].span.start))
    }

    fn parse_attribute_selector(&mut self) -> Result<AttributeSelector, SelectorParseError> {
        let (inner, end_offset) = self.take_block(
            &TokenKind::CloseSquare,
            "expected ']' to close attribute selector",
        )?;
        let mut parser = Parser {
            source: self.source,
            tokens: inner,
            pos: 0,
            end_offset,
        };
        parser.parse_attribute_contents()
    }

    fn parse_attribute_contents(&mut self) -> Result<AttributeSelector, SelectorParseError> {
        self.skip_whitespace();
        let (namespace, name) = match self.peek() {
            Some(TokenKind::Delim('|')) => {
                self.pos += 1;
                (Some(String::new()), self.parse_ident("attribute")?)
            }
            Some(TokenKind::Delim('*')) => {
                self.pos += 1;
                if self.peek() != Some(&TokenKind::Delim('|')) {
                    return self.error("expected '|' after wildcard attribute namespace");
                }
                self.pos += 1;
                (
                    Some("*".to_owned()),
                    self.parse_ident("attribute after namespace")?,
                )
            }
            _ => {
                let first = self.parse_ident("attribute")?;
                if self.peek() == Some(&TokenKind::Delim('|'))
                    && matches!(self.peek_at(1), Some(TokenKind::Ident(_)))
                {
                    self.pos += 1;
                    (Some(first), self.parse_ident("attribute after namespace")?)
                } else {
                    (None, first)
                }
            }
        };
        self.skip_whitespace();
        if self.is_eof() {
            return Ok(AttributeSelector {
                namespace,
                name,
                operator: None,
                value: None,
                modifier: None,
            });
        }

        let operator = match (self.peek(), self.peek_at(1)) {
            (Some(TokenKind::Delim('=')), _) => Some((AttributeOperator::Equals, 1)),
            (Some(TokenKind::Delim(prefix)), Some(TokenKind::Delim('='))) => match prefix {
                '~' => Some((AttributeOperator::Includes, 2)),
                '|' => Some((AttributeOperator::DashMatch, 2)),
                '^' => Some((AttributeOperator::Prefix, 2)),
                '$' => Some((AttributeOperator::Suffix, 2)),
                '*' => Some((AttributeOperator::Substring, 2)),
                _ => None,
            },
            _ => None,
        };
        let Some((operator, width)) = operator else {
            return self.error("expected attribute operator or ']'");
        };
        self.pos += width;
        self.skip_whitespace();
        let value = match self.peek() {
            Some(TokenKind::Ident(value) | TokenKind::String(value)) => value.to_string(),
            _ => return self.error("expected an identifier or string attribute value"),
        };
        self.pos += 1;
        self.skip_whitespace();
        let modifier = match self.peek() {
            Some(TokenKind::Ident(flag)) if flag.eq_ignore_ascii_case("i") => Some('i'),
            Some(TokenKind::Ident(flag)) if flag.eq_ignore_ascii_case("s") => Some('s'),
            _ => None,
        };
        if modifier.is_some() {
            self.pos += 1;
            self.skip_whitespace();
        }
        if !self.is_eof() {
            return self.error("expected ']' to close attribute selector");
        }
        Ok(AttributeSelector {
            namespace,
            name,
            operator: Some(operator),
            value: Some(value),
            modifier,
        })
    }

    fn parse_pseudo_selector(&mut self) -> Result<PseudoSelector, SelectorParseError> {
        self.pos += 1; // ':'
        let element = self.peek() == Some(&TokenKind::Colon);
        if element {
            self.pos += 1;
        }
        let (name, arguments) = match self.peek() {
            Some(TokenKind::Ident(name)) => {
                let name = name.to_string();
                self.pos += 1;
                (name, None)
            }
            Some(TokenKind::Function(name)) => {
                let name = name.to_string();
                let arguments = self.take_block(
                    &TokenKind::CloseParen,
                    "unterminated pseudo-selector argument",
                )?;
                (name, Some(arguments))
            }
            _ => return self.error("expected pseudo-selector name"),
        };
        let Some((arguments, close_offset)) = arguments else {
            return Ok(PseudoSelector {
                element,
                name,
                argument: None,
                nested_selectors: None,
            });
        };

        let lower_name = name.to_ascii_lowercase();
        let nested = match lower_name.as_str() {
            "is" | "where" | "matches" | "-webkit-any" | "-moz-any" => Some(parse_list(
                self.source,
                arguments,
                close_offset,
                false,
                true,
            )?),
            "not" | "host" | "host-context" | "slotted" => Some(parse_list(
                self.source,
                arguments,
                close_offset,
                false,
                false,
            )?),
            "has" => Some(parse_list(
                self.source,
                arguments,
                close_offset,
                true,
                false,
            )?),
            "nth-child" | "nth-last-child" => match nth_of_selector_tokens(arguments) {
                Some(selectors) => Some(parse_list(
                    self.source,
                    selectors,
                    close_offset,
                    false,
                    false,
                )?),
                None => None,
            },
            _ => None,
        };
        Ok(PseudoSelector {
            element,
            name,
            argument: Some(source_text(self.source, arguments)),
            nested_selectors: nested.map(Box::new),
        })
    }
}

/// Returns the selector-list tokens after the `of` keyword in an
/// `:nth-child()` argument, if there is one.
fn nth_of_selector_tokens<'t, 'a>(arguments: &'t [Token<'a>]) -> Option<&'t [Token<'a>]> {
    let of = arguments.iter().position(
        |token| matches!(&token.kind, TokenKind::Ident(word) if word.eq_ignore_ascii_case("of")),
    )?;
    let selectors = trim_whitespace(&arguments[of + 1..]);
    (!selectors.is_empty()).then_some(selectors)
}

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
        assert!(
            compound
                .simples
                .contains(&SimpleSelector::Class("card".into()))
        );
        assert!(
            compound
                .simples
                .contains(&SimpleSelector::Id("main".into()))
        );
    }

    #[test]
    fn parses_all_four_combinators() {
        for (input, expected) in [
            ("div p", Combinator::Descendant),
            ("div > p", Combinator::Child),
            ("h1 + p", Combinator::NextSibling),
            ("h1 ~ p", Combinator::SubsequentSibling),
        ] {
            assert_eq!(
                parse_selector_list(input).unwrap().selectors[0].combinators,
                vec![expected]
            );
        }
    }

    #[test]
    fn parses_attribute_selector() {
        let parsed = parse_selector_list("input[type=\"text\" i]").unwrap();
        let SimpleSelector::Attribute(attribute) = &parsed.selectors[0].compounds[0].simples[0]
        else {
            panic!("expected attribute")
        };
        assert_eq!(attribute.name, "type");
        assert_eq!(attribute.value.as_deref(), Some("text"));
        assert_eq!(attribute.modifier, Some('i'));
    }

    #[test]
    fn parses_wildcard_attribute_namespace() {
        let parsed = parse_selector_list("[*|href]").unwrap();
        let SimpleSelector::Attribute(attribute) = &parsed.selectors[0].compounds[0].simples[0]
        else {
            panic!("expected attribute")
        };
        assert_eq!(attribute.namespace.as_deref(), Some("*"));
        assert_eq!(attribute.name, "href");
    }

    #[test]
    fn parses_pseudo_selector_with_nested_parentheses() {
        let parsed = parse_selector_list("li:nth-child(2n + 1), a:not(.active)").unwrap();
        assert_eq!(parsed.selectors.len(), 2);
        let SimpleSelector::Pseudo(pseudo) = &parsed.selectors[0].compounds[0].simples[0] else {
            panic!("expected pseudo")
        };
        assert_eq!(pseudo.name, "nth-child");
        assert_eq!(pseudo.argument.as_deref(), Some("2n + 1"));
    }

    #[test]
    fn parses_nested_selector_functions_and_relative_has_selectors() {
        let parsed = parse_selector_list("article:has(> img.hero), :is(h1, h2, .title)").unwrap();
        assert_eq!(parsed.selectors.len(), 2);
        let SimpleSelector::Pseudo(has) = &parsed.selectors[0].compounds[0].simples[0] else {
            panic!("expected :has")
        };
        let nested = has.nested_selectors.as_ref().unwrap();
        assert_eq!(
            nested.selectors[0].leading_combinator,
            Some(Combinator::Child)
        );
        let SimpleSelector::Pseudo(is) = &parsed.selectors[1].compounds[0].simples[0] else {
            panic!("expected :is")
        };
        assert_eq!(is.nested_selectors.as_ref().unwrap().selectors.len(), 3);
    }

    #[test]
    fn parses_nth_child_of_selector_list() {
        let parsed = parse_selector_list("li:nth-child(2n + 1 of .item, #featured)").unwrap();
        let SimpleSelector::Pseudo(pseudo) = &parsed.selectors[0].compounds[0].simples[0] else {
            panic!("expected pseudo")
        };
        assert_eq!(pseudo.nested_selectors.as_ref().unwrap().selectors.len(), 2);
    }

    #[test]
    fn parses_column_combinator_and_nesting_selector() {
        let parsed = parse_selector_list("col.selected || td, & > .child").unwrap();
        assert_eq!(parsed.selectors[0].combinators, vec![Combinator::Column]);
        assert!(
            parsed.selectors[1].compounds[0]
                .simples
                .contains(&SimpleSelector::Nesting)
        );
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
