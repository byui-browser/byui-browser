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
//! Limitations: nested style rules (CSS Nesting) are reported and dropped, and
//! at-rules nested inside a style rule's block are skipped.

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
    let mut stylesheet = Stylesheet::default();
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
    let declarations = parser.consume_declaration_list(&tokens);
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
            keyframes: Vec::new(),
            block: block.map(|block| source_text(self.source, block)),
            name,
        };
        if let Some(block) = block {
            if RULE_LIST_AT_RULES.contains(&rule.name.as_str()) {
                let mut nested = Stylesheet::default();
                self.consume_rule_list(block, false, &mut nested);
                rule.rules = nested.rules;
            } else if DECLARATION_AT_RULES.contains(&rule.name.as_str()) {
                rule.declarations = self.consume_declaration_list(block);
            } else if KEYFRAMES_AT_RULES.contains(&rule.name.as_str()) {
                rule.keyframes = self.consume_keyframe_list(block);
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
            let message = format!("invalid selector: {}", error.message);
            self.error(&message, error.position);
            // The block is still consumed so its errors are reported.
            self.consume_declaration_list(block);
            return;
        }
        let declarations = self.consume_declaration_list(block);
        let selectors = split_top_level(prelude, |kind| *kind == TokenKind::Comma);
        log::trace!(
            "rule with {} selectors and {} declarations",
            selectors.len(),
            declarations.len()
        );
        for selector in selectors {
            stylesheet.rules.push(Rule {
                selector: Selector(source_text(self.source, selector)),
                declarations: declarations.clone(),
            });
        }
    }

    /// Consumes the keyframe rules of a `@keyframes` block (CSS Animations 1
    /// §3). A keyframe rule with an invalid selector list is reported and
    /// dropped; its neighbors are kept.
    fn consume_keyframe_list(&mut self, tokens: &[Token<'a>]) -> Vec<Keyframe> {
        let mut keyframes = Vec::new();
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
            let declarations = self.consume_declaration_list(block);
            match parse_keyframe_selectors(&tokens[start..index]) {
                Some(offsets) => keyframes.push(Keyframe {
                    offsets,
                    declarations,
                }),
                None => self.error("invalid keyframe selector", tokens[start].span.start),
            }
            index = end;
        }
        keyframes
    }

    /// §5.4.5 "consume a list of declarations".
    fn consume_declaration_list(&mut self, tokens: &[Token<'a>]) -> Vec<Declaration> {
        let mut declarations = Vec::new();
        let mut index = 0;
        while index < tokens.len() {
            match &tokens[index].kind {
                TokenKind::Whitespace | TokenKind::Semicolon => index += 1,
                TokenKind::AtKeyword(name) => {
                    log::debug!("skipping @{name} nested in a declaration block");
                    let mut ignored = Stylesheet::default();
                    index = self.consume_at_rule(tokens, index, &mut ignored);
                }
                TokenKind::Ident(_) => {
                    let end = find_semicolon(tokens, index);
                    if let Some(declaration) = self.consume_declaration(&tokens[index..end]) {
                        declarations.push(declaration);
                    }
                    index = end;
                }
                _ => {
                    // Anything else starts a nested style rule (CSS Nesting),
                    // which this engine does not apply. Skip the whole rule.
                    let offset = tokens[index].span.start;
                    let mut end = index;
                    while end < tokens.len()
                        && !matches!(
                            tokens[end].kind,
                            TokenKind::Semicolon | TokenKind::OpenCurly
                        )
                    {
                        end = skip_component_value(tokens, end);
                    }
                    if tokens.get(end).map(|token| &token.kind) == Some(&TokenKind::OpenCurly) {
                        self.error("nested style rules are not supported", offset);
                        end = skip_component_value(tokens, end);
                    } else {
                        self.error("declaration must start with a property name", offset);
                    }
                    index = end;
                }
            }
        }
        declarations
    }

    /// §5.4.6 "consume a declaration" from tokens up to (not including) `;`.
    fn consume_declaration(&mut self, tokens: &[Token<'a>]) -> Option<Declaration> {
        let offset = tokens[0].span.start;
        let TokenKind::Ident(name) = &tokens[0].kind else {
            unreachable!("declarations start with an ident");
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
        if value.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_strings_functions_and_important_values() {
        let sheet = parse_stylesheet(
            r#"/* comment */ body, .hero { background: url("a;b.png"); content: "}"; color: rgb(1, 2, 3) !important; }"#,
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selector, Selector("body".into()));
        assert_eq!(sheet.rules[1].selector, Selector(".hero".into()));
        assert_eq!(sheet.rules[0].declarations.len(), 3);
        assert_eq!(sheet.rules[0].declarations[0].value, r#"url("a;b.png")"#);
        assert_eq!(sheet.rules[0].declarations[1].value, r#""}""#);
        assert_eq!(
            sheet.rules[0].declarations[2].value,
            "rgb(1, 2, 3) !important"
        );
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parses_at_rules_and_nested_rules() {
        let sheet = parse_stylesheet(
            "@media screen and (min-width: 10px) { body { color: red; } } @import url('x.css');",
        );
        assert_eq!(sheet.at_rules.len(), 2);
        assert_eq!(sheet.at_rules[0].name, "media");
        assert_eq!(sheet.at_rules[0].prelude, "screen and (min-width: 10px)");
        assert_eq!(sheet.at_rules[0].rules.len(), 1);
        assert_eq!(sheet.at_rules[1].prelude, "url('x.css')");
        assert!(sheet.rules.is_empty());
    }

    #[test]
    fn parses_font_face_declarations() {
        let sheet = parse_stylesheet("@font-face { font-family: X; src: url(x.woff2) }");
        assert_eq!(sheet.at_rules[0].declarations.len(), 2);
        assert_eq!(sheet.at_rules[0].declarations[1].value, "url(x.woff2)");
    }

    #[test]
    fn parses_keyframes_blocks() {
        let (sheet, errors) = parse_stylesheet_with_errors(
            "@keyframes slide { from { margin-left: 0 } 50%, 75% { margin-left: 10px } \
             120% { color: red } TO { margin-left: 20px; color: blue } } p { color: red }",
        );
        let rule = &sheet.at_rules[0];
        assert_eq!(rule.name, "keyframes");
        assert_eq!(rule.prelude, "slide");
        let offsets: Vec<_> = rule.keyframes.iter().map(|k| k.offsets.clone()).collect();
        assert_eq!(offsets, vec![vec![0.0], vec![0.5, 0.75], vec![1.0]]);
        assert_eq!(rule.keyframes[2].declarations.len(), 2);
        // `120%` is out of range, so only that keyframe rule is dropped.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "invalid keyframe selector");
        assert_eq!(sheet.rules.len(), 1);

        let prefixed = parse_stylesheet("@-webkit-keyframes x { to { color: red } }");
        assert_eq!(prefixed.at_rules[0].keyframes.len(), 1);
    }

    #[test]
    fn recovers_from_bad_declaration() {
        let (sheet, errors) =
            parse_stylesheet_with_errors("p { color: red; broken; background: blue; }");
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].offset, 16);
    }

    #[test]
    fn invalid_selector_drops_the_whole_rule_but_keeps_neighbors() {
        let (sheet, errors) =
            parse_stylesheet_with_errors("p, div > { color: red } #1 { color: red } a { b: c }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector, Selector("a".into()));
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn values_keep_author_text_without_comments() {
        let sheet = parse_stylesheet("p { margin: 1px/* a */2px  3px; --Brand: { x } }");
        assert_eq!(sheet.rules[0].declarations[0].value, "1px 2px  3px");
        assert_eq!(sheet.rules[0].declarations[1].property, "--Brand");
        assert_eq!(sheet.rules[0].declarations[1].value, "{ x }");
    }

    #[test]
    fn reports_unclosed_blocks_and_unfinished_rules() {
        let (sheet, errors) = parse_stylesheet_with_errors("p { color: red");
        assert_eq!(sheet.rules[0].declarations.len(), 1);
        assert_eq!(errors[0].message, "unclosed `{` in CSS rule");

        let (sheet, errors) = parse_stylesheet_with_errors("a { color: red } p");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(errors[0].offset, 17);
    }

    #[test]
    fn skips_nested_rules_inside_declaration_blocks() {
        let (sheet, errors) =
            parse_stylesheet_with_errors("p { color: red; &:hover { color: blue } margin: 0 }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn ignores_html_comment_markers_at_top_level() {
        let sheet = parse_stylesheet("<!-- p { color: red } -->");
        assert_eq!(sheet.rules.len(), 1);
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parses_inline_style_declarations() {
        let mut sheet = Stylesheet::default();
        let declarations = parse_declarations("color: red !IMPORTANT; ; width:", &mut sheet, 10);
        assert_eq!(declarations.len(), 1);
        assert_eq!(declarations[0].value, "red !important");
        assert_eq!(sheet.errors[0].offset, 35);
    }

    #[test]
    fn malformed_input_never_panics() {
        // Deterministic pseudo-random soup of CSS-significant characters,
        // including multi-byte text, escapes, and unbalanced brackets.
        const PIECES: &[&str] = &[
            "{",
            "}",
            "(",
            ")",
            "[",
            "]",
            ";",
            ":",
            ",",
            "\"",
            "'",
            "\\",
            "/*",
            "*/",
            "@",
            "#",
            ".",
            "!",
            "important",
            "url(",
            "a",
            "-",
            "--x",
            "1",
            "2.5e",
            "%",
            "px",
            "é",
            "日本",
            " ",
            "\n",
            "\r",
            "\0",
            "<!--",
            "-->",
            ">",
            "+",
            "~",
            "|",
            "=",
            "*",
            "&",
            "::",
            "is(",
            "nth-child(",
            "\\31 ",
        ];
        let mut state = 0x2545_f491_u32;
        for _ in 0..2_000 {
            let mut input = String::new();
            for _ in 0..(state % 40) {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                input.push_str(PIECES[state as usize % PIECES.len()]);
            }
            let sheet = parse_stylesheet(&input);
            let _ = crate::parse_selector_list(&input);
            let document = html::parse_raw_html("<p class='a' id='b'><a href=x>y</a></p>".into());
            let _ = crate::style_document(&document, &[sheet]);
        }
    }
}
