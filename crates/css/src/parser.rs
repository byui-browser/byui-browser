//! Forgiving CSS stylesheet parser built on the [`crate::tokenizer`].
//!
//! The parser follows the rule and declaration algorithms of CSS Syntax Level
//! 3 §5 ("Parsing"): it walks the token stream, consumes at-rules, qualified
//! rules, and declaration lists, and recovers at the points the specification
//! defines. Valid rules are kept when a neighboring rule is malformed, and the
//! malformed pieces are reported as [`ParseError`] values whose offsets are
//! byte offsets into the original input.
//!
//! Each style rule's prelude is parsed as a selector list. Per Selectors 4 §3.1
//! a rule whose selector list is invalid is dropped as a whole. A valid list is
//! stored as one [`Rule`] per complex selector, all sharing the declarations.
//!
//! `@keyframes` blocks (and the `-webkit-`/`-moz-` aliases) are parsed into
//! [`Keyframe`] rules per CSS Animations 1 §3.
//!
//! Nested style rules and nested group at-rules are retained in the stylesheet
//! tree so the cascade can expand them with the correct parent selector.

use crate::selector::parse_selector_tokens;
use crate::stylesheet::{AtRule, Declaration, Keyframe, ParseError, Rule, Selector, Stylesheet};
use crate::tokenizer::{Token, TokenKind, source_text, tokenize, trim_whitespace};

/// Parse CSS and retain all valid rules while collecting recoverable errors.
pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let (stylesheet, _) = parse_stylesheet_with_errors(input);
    stylesheet
}

/// Parse CSS and return both the useful AST and diagnostics.
///
/// The diagnostics are also stored in [`Stylesheet::errors`].
pub fn parse_stylesheet_with_errors(input: &str) -> (Stylesheet, Vec<ParseError>) {
    log::debug!("parsing stylesheet: {} bytes", input.len());
    let tokens = tokenize(input);
    let mut parser = Parser {
        source: input,
        errors: Vec::new(),
    };
    let mut stylesheet = Stylesheet {
        source: input.to_owned(),
        ..Stylesheet::default()
    };
    parser.consume_rule_list(&tokens, true, &mut stylesheet);
    stylesheet.errors = parser.errors;
    let errors = stylesheet.errors.clone();
    log::debug!(
        "parsed stylesheet: {} tokens, {} rules, {} at-rules, {} errors",
        tokens.len(),
        stylesheet.rules.len(),
        stylesheet.at_rules.len(),
        errors.len()
    );
    (stylesheet, errors)
}

/// Parses the contents of a declaration block, such as a `style` attribute.
///
/// Errors are appended to `stylesheet.errors`, offset by `offset` bytes.
pub(crate) fn parse_declarations(
    input: &str,
    stylesheet: &mut Stylesheet,
    offset: usize,
) -> Vec<Declaration> {
    let tokens = tokenize(input);
    let mut parser = Parser {
        source: input,
        errors: Vec::new(),
    };
    let declarations = parser.consume_declaration_list(&tokens).declarations;
    stylesheet
        .errors
        .extend(parser.errors.into_iter().map(|error| ParseError {
            message: error.message,
            offset: error.offset + offset,
        }));
    declarations
}

/// At-rules whose block holds a nested rule list, rather than declarations.
const RULE_LIST_AT_RULES: [&str; 6] = [
    "media",
    "supports",
    "layer",
    "document",
    "container",
    "scope",
];

/// At-rules whose block is a declaration list.
const DECLARATION_AT_RULES: [&str; 6] = [
    "font-face",
    "page",
    "property",
    "counter-style",
    "font-palette-values",
    "view-transition",
];

/// At-rules whose block is a list of keyframe rules.
const KEYFRAMES_AT_RULES: [&str; 3] = ["keyframes", "-webkit-keyframes", "-moz-keyframes"];

struct Parser<'a> {
    source: &'a str,
    errors: Vec<ParseError>,
}

/// Parsed contents of a declaration/style block, retaining CSS nesting.
#[derive(Default)]
struct DeclarationBlock {
    declarations: Vec<Declaration>,
    nested_rules: Vec<Rule>,
    nested_at_rules: Vec<AtRule>,
}

/// Returns the index just past the component value starting at `index`.
///
/// A component value is a single token, or a whole `()`, `[]`, `{}` block or
/// function including its closing token. An unclosed block runs to the end.
pub(crate) fn skip_component_value(tokens: &[Token<'_>], index: usize) -> usize {
    let closing = match tokens[index].kind {
        TokenKind::OpenCurly => TokenKind::CloseCurly,
        TokenKind::OpenSquare => TokenKind::CloseSquare,
        TokenKind::OpenParen | TokenKind::Function(_) => TokenKind::CloseParen,
        _ => return index + 1,
    };
    // Nested blocks need their own matching closers, so track a stack.
    let mut stack = vec![closing];
    let mut position = index + 1;
    while position < tokens.len() {
        let kind = &tokens[position].kind;
        match kind {
            TokenKind::OpenCurly => stack.push(TokenKind::CloseCurly),
            TokenKind::OpenSquare => stack.push(TokenKind::CloseSquare),
            TokenKind::OpenParen | TokenKind::Function(_) => stack.push(TokenKind::CloseParen),
            TokenKind::CloseCurly | TokenKind::CloseSquare | TokenKind::CloseParen
                if stack.last() == Some(kind) =>
            {
                stack.pop();
                if stack.is_empty() {
                    return position + 1;
                }
            }
            _ => {}
        }
        position += 1;
    }
    tokens.len()
}

/// Splits `tokens` at top-level tokens for which `is_separator` is true.
pub(crate) fn split_top_level<'t, 'a>(
    tokens: &'t [Token<'a>],
    is_separator: impl Fn(&TokenKind<'a>) -> bool,
) -> Vec<&'t [Token<'a>]> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < tokens.len() {
        if is_separator(&tokens[index].kind) {
            pieces.push(&tokens[start..index]);
            start = index + 1;
            index += 1;
        } else {
            index = skip_component_value(tokens, index);
        }
    }
    pieces.push(&tokens[start..]);
    pieces
}

impl<'a> Parser<'a> {
    fn error(&mut self, message: &str, offset: usize) {
        log::debug!("css parse error at offset {offset}: {message}");
        self.errors.push(ParseError {
            message: message.to_owned(),
            offset,
        });
    }

    /// §5.4.1 "consume a list of rules".
    fn consume_rule_list(
        &mut self,
        tokens: &[Token<'a>],
        top_level: bool,
        stylesheet: &mut Stylesheet,
    ) {
        let mut index = 0;
        while index < tokens.len() {
            match &tokens[index].kind {
                TokenKind::Whitespace => index += 1,
                // HTML comment markers are ignored around top-level rules.
                TokenKind::Cdo | TokenKind::Cdc if top_level => index += 1,
                TokenKind::AtKeyword(_) => {
                    index = self.consume_at_rule(tokens, index, stylesheet);
                }
                _ => index = self.consume_qualified_rule(tokens, index, stylesheet),
            }
        }
    }

    /// §5.4.2 "consume an at-rule". Returns the index after the rule.
    fn consume_at_rule(
        &mut self,
        tokens: &[Token<'a>],
        start: usize,
        stylesheet: &mut Stylesheet,
    ) -> usize {
        let TokenKind::AtKeyword(name) = &tokens[start].kind else {
            unreachable!("consume_at_rule starts at an at-keyword");
        };
        let name = name.to_ascii_lowercase();
        let mut index = start + 1;
        let prelude_start = index;
        loop {
            match tokens.get(index).map(|token| &token.kind) {
                None => {
                    let prelude = &tokens[prelude_start..];
                    stylesheet
                        .at_rules
                        .push(self.build_at_rule(name, prelude, None));
                    return tokens.len();
                }
                Some(TokenKind::Semicolon) => {
                    let prelude = &tokens[prelude_start..index];
                    stylesheet
                        .at_rules
                        .push(self.build_at_rule(name, prelude, None));
                    return index + 1;
                }
                Some(TokenKind::OpenCurly) => {
                    let end = skip_component_value(tokens, index);
                    let prelude = &tokens[prelude_start..index];
                    let block = block_contents(tokens, index, end);
                    if !closes_block(tokens, end) {
                        self.error("unclosed `{` in CSS rule", tokens[index].span.start);
                    }
                    let rule = self.build_at_rule(name, prelude, Some(block));
                    stylesheet.at_rules.push(rule);
                    return end;
                }
                Some(_) => index = skip_component_value(tokens, index),
            }
        }
    }

    fn build_at_rule(
        &mut self,
        name: String,
        prelude: &[Token<'a>],
        block: Option<&[Token<'a>]>,
    ) -> AtRule {
        log::trace!("at-rule @{name} (block={})", block.is_some());
        let mut rule = AtRule {
            prelude: source_text(self.source, prelude),
            rules: Vec::new(),
            declarations: Vec::new(),
            at_rules: Vec::new(),
            keyframes: Vec::new(),
            block: block.map(|block| source_text(self.source, block)),
            name,
        };
        if let Some(block) = block {
            if RULE_LIST_AT_RULES.contains(&rule.name.as_str()) {
                let mut nested = Stylesheet::default();
                self.consume_rule_list(block, false, &mut nested);
                rule.rules = nested.rules;
                rule.at_rules = nested.at_rules;
                self.errors.extend(nested.errors);
            } else if DECLARATION_AT_RULES.contains(&rule.name.as_str()) {
                let parsed = self.consume_declaration_list(block);
                rule.declarations = parsed.declarations;
                rule.at_rules = parsed.nested_at_rules;
            } else if KEYFRAMES_AT_RULES.contains(&rule.name.as_str()) {
                let (keyframes, rules) = self.consume_keyframe_list(block);
                rule.keyframes = keyframes;
                rule.rules = rules;
            }
        }
        rule
    }

    /// §5.4.3 "consume a qualified rule". Returns the index after the rule.
    fn consume_qualified_rule(
        &mut self,
        tokens: &[Token<'a>],
        start: usize,
        stylesheet: &mut Stylesheet,
    ) -> usize {
        let mut index = start;
        while index < tokens.len() {
            if tokens[index].kind == TokenKind::OpenCurly {
                let end = skip_component_value(tokens, index);
                if !closes_block(tokens, end) {
                    self.error("unclosed `{` in CSS rule", tokens[index].span.start);
                }
                let prelude = &tokens[start..index];
                let block = block_contents(tokens, index, end);
                self.build_style_rule(prelude, block, stylesheet);
                return end;
            }
            index = skip_component_value(tokens, index);
        }
        self.error(
            "unfinished CSS rule; expected `{`",
            tokens[start].span.start,
        );
        tokens.len()
    }

    fn build_style_rule(
        &mut self,
        prelude: &[Token<'a>],
        block: &[Token<'a>],
        stylesheet: &mut Stylesheet,
    ) {
        let prelude = trim_whitespace(prelude);
        let offset = prelude.first().map_or(0, |token| token.span.start);
        if prelude.is_empty() {
            self.error("a style rule needs a selector", offset);
            self.consume_declaration_list(block);
            return;
        }
        if let Err(error) = parse_selector_tokens(self.source, prelude) {
            self.error(
                &format!("unrecognized or invalid selector syntax: {}", error.message),
                error.position,
            );
        }
        let parsed = self.consume_declaration_list(block);
        let selectors = split_top_level(prelude, |kind| *kind == TokenKind::Comma);
        for selector in selectors {
            stylesheet.rules.push(Rule {
                selector: Selector(source_text(self.source, selector)),
                declarations: parsed.declarations.clone(),
                nested_rules: parsed.nested_rules.clone(),
                nested_at_rules: parsed.nested_at_rules.clone(),
            });
        }
    }

    fn consume_declaration_list(&mut self, tokens: &[Token<'a>]) -> DeclarationBlock {
        let mut parsed = DeclarationBlock::default();
        let mut index = 0;
        while index < tokens.len() {
            match &tokens[index].kind {
                TokenKind::Whitespace | TokenKind::Semicolon => index += 1,
                TokenKind::AtKeyword(_) => {
                    let mut nested = Stylesheet::default();
                    index = self.consume_at_rule(tokens, index, &mut nested);
                    parsed.nested_at_rules.extend(nested.at_rules);
                    parsed.nested_rules.extend(nested.rules);
                }
                _ => {
                    // Find the next top-level semicolon or opening brace. A
                    // brace means this item is a nested qualified rule;
                    // otherwise an identifier followed by ':' is a declaration.
                    let mut end = index;
                    while end < tokens.len()
                        && !matches!(
                            tokens[end].kind,
                            TokenKind::Semicolon | TokenKind::OpenCurly
                        )
                    {
                        end = skip_component_value(tokens, end);
                    }

                    let custom_property_block = matches!(
                        &tokens[index].kind,
                        TokenKind::Ident(name) if name.starts_with("--")
                    ) && tokens[index..end]
                        .iter()
                        .any(|token| token.kind == TokenKind::Colon);

                    if tokens.get(end).map(|token| &token.kind) == Some(&TokenKind::OpenCurly)
                        && !custom_property_block
                    {
                        let rule_end = skip_component_value(tokens, end);
                        if !closes_block(tokens, rule_end) {
                            self.error("unclosed `{` in nested CSS rule", tokens[end].span.start);
                        }
                        let prelude = trim_whitespace(&tokens[index..end]);
                        if prelude.is_empty() {
                            self.error(
                                "a nested style rule needs a selector",
                                tokens[index].span.start,
                            );
                        } else {
                            if let Err(error) = parse_selector_tokens(self.source, prelude) {
                                let message = format!(
                                    "unrecognized or invalid nested selector syntax: {}",
                                    error.message
                                );
                                self.error(&message, error.position);
                            }
                            let block = block_contents(tokens, end, rule_end);
                            let children = self.consume_declaration_list(block);
                            let selectors =
                                split_top_level(prelude, |kind| *kind == TokenKind::Comma);
                            for selector in selectors {
                                parsed.nested_rules.push(Rule {
                                    selector: Selector(source_text(self.source, selector)),
                                    declarations: children.declarations.clone(),
                                    nested_rules: children.nested_rules.clone(),
                                    nested_at_rules: children.nested_at_rules.clone(),
                                });
                            }
                        }
                        index = rule_end;
                    } else if custom_property_block {
                        // Custom properties accept arbitrary component values,
                        // including balanced curly blocks. Consume through the
                        // next top-level semicolon rather than treating `{` as
                        // the beginning of a nested selector.
                        let declaration_end = find_semicolon(tokens, index);
                        if let Some(declaration) =
                            self.consume_declaration(&tokens[index..declaration_end])
                        {
                            parsed.declarations.push(declaration);
                        }
                        index = if declaration_end < tokens.len() {
                            declaration_end + 1
                        } else {
                            declaration_end
                        };
                    } else {
                        if matches!(tokens[index].kind, TokenKind::Ident(_)) {
                            if let Some(declaration) = self.consume_declaration(&tokens[index..end])
                            {
                                parsed.declarations.push(declaration);
                            }
                        } else {
                            self.error(
                                "declaration must start with a property name",
                                tokens[index].span.start,
                            );
                        }
                        index = if end < tokens.len() { end + 1 } else { end };
                    }
                }
            }
        }
        parsed
    }

    /// Â§5.4.6 "consume a declaration" from tokens up to (not including) `;`.
    fn consume_keyframe_list(&mut self, tokens: &[Token<'a>]) -> (Vec<Keyframe>, Vec<Rule>) {
        let mut keyframes = Vec::new();
        let mut rules = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            match &tokens[index].kind {
                TokenKind::Whitespace | TokenKind::Semicolon => {
                    index += 1;
                    continue;
                }
                TokenKind::AtKeyword(name) => {
                    log::debug!("skipping @{name} inside @keyframes");
                    let mut ignored = Stylesheet::default();
                    index = self.consume_at_rule(tokens, index, &mut ignored);
                    continue;
                }
                _ => {}
            }
            let start = index;
            while index < tokens.len() && tokens[index].kind != TokenKind::OpenCurly {
                index = skip_component_value(tokens, index);
            }
            if index >= tokens.len() {
                self.error(
                    "unfinished keyframe rule; expected `{`",
                    tokens[start].span.start,
                );
                break;
            }
            let end = skip_component_value(tokens, index);
            if !closes_block(tokens, end) {
                self.error("unclosed `{` in keyframe rule", tokens[index].span.start);
            }
            let block = block_contents(tokens, index, end);
            let declarations = self.consume_declaration_list(block).declarations;
            match parse_keyframe_selectors(&tokens[start..index]) {
                Some(offsets) => {
                    keyframes.push(Keyframe {
                        offsets,
                        declarations: declarations.clone(),
                    });
                    rules.push(Rule {
                        selector: Selector(source_text(self.source, &tokens[start..index])),
                        declarations,
                        nested_rules: Vec::new(),
                        nested_at_rules: Vec::new(),
                    });
                }
                None => self.error("invalid keyframe selector", tokens[start].span.start),
            }
            index = end;
        }
        (keyframes, rules)
    }

    fn consume_declaration(&mut self, tokens: &[Token<'a>]) -> Option<Declaration> {
        let first = tokens.first()?;
        let offset = first.span.start;
        let TokenKind::Ident(name) = &first.kind else {
            self.error("declaration property must be an identifier", offset);
            return None;
        };
        let mut index = 1;
        while tokens.get(index).map(|token| &token.kind) == Some(&TokenKind::Whitespace) {
            index += 1;
        }
        if tokens.get(index).map(|token| &token.kind) != Some(&TokenKind::Colon) {
            self.error("declaration is missing `:`", offset);
            return None;
        }
        let mut value = trim_whitespace(&tokens[index + 1..]);
        let important = {
            let mut meaningful = value
                .iter()
                .rev()
                .filter(|token| token.kind != TokenKind::Whitespace);
            matches!(
                (meaningful.next().map(|t| &t.kind), meaningful.next().map(|t| &t.kind)),
                (Some(TokenKind::Ident(word)), Some(TokenKind::Delim('!')))
                    if word.eq_ignore_ascii_case("important")
            )
        };
        if important {
            let bang = value
                .iter()
                .rposition(|token| token.kind == TokenKind::Delim('!'))
                .expect("important values contain `!`");
            value = trim_whitespace(&value[..bang]);
        }
        if value.is_empty() && !name.starts_with("--") {
            self.error("declaration has an empty property or value", offset);
            return None;
        }
        // Custom property names are case-sensitive; all others are not.
        let property = if name.starts_with("--") {
            name.to_string()
        } else {
            name.to_ascii_lowercase()
        };
        let mut value = source_text(self.source, value);
        if important {
            value.push_str(" !important");
        }
        log::trace!("declaration `{property}: {value}`");
        Some(Declaration { property, value })
    }
}

/// Parses a `<keyframe-selector>#` list: `from`, `to`, or percentages from
/// `0%` to `100%`, returned as fractions of an iteration.
fn parse_keyframe_selectors(tokens: &[Token<'_>]) -> Option<Vec<f32>> {
    split_top_level(tokens, |kind| *kind == TokenKind::Comma)
        .into_iter()
        .map(|selector| match trim_whitespace(selector) {
            [token] => match &token.kind {
                TokenKind::Ident(word) if word.eq_ignore_ascii_case("from") => Some(0.0),
                TokenKind::Ident(word) if word.eq_ignore_ascii_case("to") => Some(1.0),
                TokenKind::Percentage(percent) if (0.0..=100.0).contains(percent) => {
                    Some((*percent / 100.0) as f32)
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Returns the tokens between a `{` at `open` and its closing `}`.
fn block_contents<'t, 'a>(tokens: &'t [Token<'a>], open: usize, end: usize) -> &'t [Token<'a>] {
    let close = if closes_block(tokens, end) {
        end - 1
    } else {
        end
    };
    &tokens[open + 1..close.max(open + 1)]
}

/// Whether the component value ending at `end` was closed by a `}`.
fn closes_block(tokens: &[Token<'_>], end: usize) -> bool {
    end > 0 && end <= tokens.len() && tokens[end - 1].kind == TokenKind::CloseCurly
}

/// Returns the index of the next top-level `;` at or after `start`, or the end.
fn find_semicolon(tokens: &[Token<'_>], start: usize) -> usize {
    let mut index = start;
    while index < tokens.len() && tokens[index].kind != TokenKind::Semicolon {
        index = skip_component_value(tokens, index);
    }
    index
}
