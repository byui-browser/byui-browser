//! CSS parsing, cascade, inheritance, and computed styles.
//!
//! The parser is intentionally forgiving, like a browser parser: it keeps
//! valid rules when a neighboring rule is malformed and reports malformed
//! pieces as [`ParseError`] values.

#![forbid(unsafe_code)]

/// Parsing of CSS selector syntax into a structured abstract syntax tree.
pub mod selector;
pub mod shorthands;
pub mod values;
pub use selector::{
    AttributeOperator, AttributeSelector, Combinator, ComplexSelector, CompoundSelector,
    PseudoSelector, SelectorList, SelectorParseError, SimpleSelector, TypeSelector,
    parse_selector_list,
};

use html::{HTMLDocument, NodeID, NodeKind};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity(pub u32, pub u32, pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
}

/// An at-rule such as `@media`, `@import`, or `@font-face`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtRule {
    pub name: String,
    pub prelude: String,
    pub rules: Vec<Rule>,
    pub declarations: Vec<Declaration>,
    pub block: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    pub at_rules: Vec<AtRule>,
    pub errors: Vec<ParseError>,
}

/// CSS's view of the HTML team's arena-backed DOM.
pub type Dom = HTMLDocument;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    pub styles: Vec<(NodeID, Vec<Declaration>)>,
}

/// An sRGB color with 8-bit red, green, blue, and alpha channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssColor {
    /// Red channel, from 0 to 255.
    pub r: u8,
    /// Green channel, from 0 to 255.
    pub g: u8,
    /// Blue channel, from 0 to 255.
    pub b: u8,
    /// Alpha channel, from 0 (transparent) to 255 (opaque).
    pub a: u8,
}

/// Parses the solid color forms supported by the first renderer slice.
///
/// Supports `#rgb`, `#rrggbb`, `rgb(r, g, b)` with integer channels, and
/// common opaque named colors. Alpha and other CSS color forms return `None`.
pub fn parse_color(value: &str) -> Option<CssColor> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.is_ascii() {
            return None;
        }
        let (r, g, b) = match hex.len() {
            3 => {
                let mut digits = hex.chars();
                let channel = |digit: char| digit.to_digit(16).map(|value| (value * 17) as u8);
                (
                    channel(digits.next()?)?,
                    channel(digits.next()?)?,
                    channel(digits.next()?)?,
                )
            }
            6 => (
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ),
            _ => return None,
        };
        return Some(CssColor { r, g, b, a: 255 });
    }
    let lower = value.to_ascii_lowercase();
    if let Some(channels) = lower
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts = channels.split(',').map(str::trim).collect::<Vec<_>>();
        if let [r, g, b] = parts.as_slice() {
            return Some(CssColor {
                r: r.parse().ok()?,
                g: g.parse().ok()?,
                b: b.parse().ok()?,
                a: 255,
            });
        }
        return None;
    }
    let (r, g, b, a) = match lower.as_str() {
        "black" => (0, 0, 0, 255),
        "silver" => (192, 192, 192, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "white" => (255, 255, 255, 255),
        "maroon" => (128, 0, 0, 255),
        "red" => (255, 0, 0, 255),
        "purple" => (128, 0, 128, 255),
        "fuchsia" => (255, 0, 255, 255),
        "green" => (0, 128, 0, 255),
        "lime" => (0, 255, 0, 255),
        "olive" => (128, 128, 0, 255),
        "yellow" => (255, 255, 0, 255),
        "navy" => (0, 0, 128, 255),
        "blue" => (0, 0, 255, 255),
        "teal" => (0, 128, 128, 255),
        "aqua" => (0, 255, 255, 255),
        "orange" => (255, 165, 0, 255),
        "rebeccapurple" => (102, 51, 153, 255),
        _ => return None,
    };
    Some(CssColor { r, g, b, a })
}

impl ComputedStyles {
    /// Returns the computed declarations for an element, or `None` for nodes
    /// that are not elements.
    ///
    /// Entries are stored in document order, so lookup is a binary search.
    pub fn declarations_for(&self, node: NodeID) -> Option<&[Declaration]> {
        computed_style(&self.styles, node).map(Vec::as_slice)
    }

    /// Returns the computed value text of one property for an element.
    pub fn value_for(&self, node: NodeID, property: &str) -> Option<&str> {
        self.declarations_for(node)?
            .iter()
            .find(|declaration| declaration.property == property)
            .map(|declaration| declaration.value.as_str())
    }

    /// Returns a parsed color property for an HTML node after the cascade.
    ///
    /// `property` is typically `color` or `background-color`. Unsupported
    /// color syntax and absent declarations return `None`.
    pub fn color_for(&self, node: NodeID, property: &str) -> Option<CssColor> {
        self.value_for(node, property).and_then(parse_color)
    }
}

/// Returns the bundled user-agent stylesheet parsed from `user_agent.css`.
///
/// The file is embedded at build time and parsed once.
pub fn user_agent_stylesheet() -> &'static Stylesheet {
    static SHEET: OnceLock<Stylesheet> = OnceLock::new();
    SHEET.get_or_init(|| {
        let sheet = parse_stylesheet(include_str!("user_agent.css"));
        assert!(sheet.errors.is_empty(), "bundled user-agent CSS must parse");
        sheet
    })
}

/// Returns a supported color default from the bundled `html` user-agent rule.
///
/// Callers use `color` for the initial text color. The rule deliberately has no
/// `background-color`: per CSS Backgrounds 3 §2.11.2 the canvas takes the root
/// or `body` background, and the user agent's default canvas color is applied
/// by `paint`. Missing or unsupported properties return `None`.
pub fn user_agent_default_color(property: &str) -> Option<CssColor> {
    user_agent_stylesheet()
        .rules
        .iter()
        .find(|rule| rule.selector.0 == "html")?
        .declarations
        .iter()
        .find(|declaration| declaration.property == property)
        .and_then(|declaration| parse_color(&declaration.value))
}

/// Parse CSS and retain all valid rules while collecting recoverable errors.
pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let (stylesheet, _) = parse_stylesheet_with_errors(input);
    stylesheet
}

/// Parse CSS and return both the useful AST and diagnostics.
pub fn parse_stylesheet_with_errors(input: &str) -> (Stylesheet, Vec<ParseError>) {
    let source = remove_comments(input);
    let mut stylesheet = Stylesheet::default();
    parse_rule_list(&source, &mut stylesheet, None);
    let errors = stylesheet.errors.clone();
    (stylesheet, errors)
}

fn parse_rule_list(source: &str, stylesheet: &mut Stylesheet, base_offset: Option<usize>) {
    let mut cursor = 0;
    while cursor < source.len() {
        cursor += source[cursor..]
            .chars()
            .take_while(|character| character.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
        if cursor >= source.len() {
            break;
        }

        let Some((boundary, delimiter)) = find_boundary(&source[cursor..]) else {
            stylesheet.errors.push(ParseError {
                message: "unfinished CSS rule; expected `;` or `{`".into(),
                offset: base_offset.unwrap_or(0) + cursor,
            });
            break;
        };
        let boundary = cursor + boundary;
        let prelude = source[cursor..boundary].trim();
        if prelude.is_empty() {
            cursor = boundary + delimiter.len_utf8();
            continue;
        }

        match delimiter {
            ';' => {
                if prelude.starts_with('@') {
                    stylesheet.at_rules.push(parse_at_rule(prelude, None));
                } else {
                    stylesheet.errors.push(ParseError {
                        message: "a style rule needs a declaration block".into(),
                        offset: base_offset.unwrap_or(0) + cursor,
                    });
                }
                cursor = boundary + 1;
            }
            '{' => {
                let block_start = boundary + 1;
                let Some(block_end) = matching_brace(source, boundary) else {
                    stylesheet.errors.push(ParseError {
                        message: "unclosed `{` in CSS rule".into(),
                        offset: base_offset.unwrap_or(0) + boundary,
                    });
                    break;
                };
                let block = &source[block_start..block_end];
                if prelude.starts_with('@') {
                    stylesheet
                        .at_rules
                        .push(parse_at_rule(prelude, Some(block)));
                } else {
                    let declarations = parse_declarations(
                        block,
                        stylesheet,
                        base_offset.unwrap_or(0) + block_start,
                    );
                    for selector in split_selectors(prelude) {
                        stylesheet.rules.push(Rule {
                            selector: Selector(selector),
                            declarations: declarations.clone(),
                        });
                    }
                }
                cursor = block_end + 1;
            }
            '}' => {
                stylesheet.errors.push(ParseError {
                    message: "unexpected `}`".into(),
                    offset: base_offset.unwrap_or(0) + boundary,
                });
                cursor = boundary + 1;
            }
            _ => unreachable!(),
        }
    }
}

fn parse_at_rule(prelude: &str, block: Option<&str>) -> AtRule {
    let mut pieces = prelude[1..].splitn(2, char::is_whitespace);
    let name = pieces.next().unwrap_or_default().to_ascii_lowercase();
    let prelude = pieces.next().unwrap_or_default().trim().to_owned();
    let mut rule = AtRule {
        name,
        prelude,
        rules: Vec::new(),
        declarations: Vec::new(),
        block: block.map(str::to_owned),
    };
    if let Some(block) = block {
        if matches!(
            rule.name.as_str(),
            "media" | "supports" | "layer" | "document" | "container"
        ) {
            let mut nested = Stylesheet::default();
            parse_rule_list(block, &mut nested, None);
            rule.rules = nested.rules;
        } else {
            let mut nested = Stylesheet::default();
            rule.declarations = parse_declarations(block, &mut nested, 0);
        }
    }
    rule
}

fn find_boundary(source: &str) -> Option<(usize, char)> {
    let mut quote = None;
    let mut escaped = false;
    let mut parentheses: usize = 0;
    let mut brackets: usize = 0;
    for (index, character) in source.char_indices() {
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            ';' | '{' | '}' if parentheses == 0 && brackets == 0 => {
                return Some((index, character));
            }
            _ => {}
        }
    }
    None
}

fn matching_brace(source: &str, opening: usize) -> Option<usize> {
    let mut depth = 1;
    let mut quote = None;
    let mut escaped = false;
    for (relative, character) in source[opening + 1..].char_indices() {
        let index = opening + 1 + relative;
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_selectors(input: &str) -> Vec<String> {
    split_top_level(input, ',')
        .into_iter()
        .map(|selector| selector.trim().to_owned())
        .filter(|selector| !selector.is_empty())
        .collect()
}

fn parse_declarations(input: &str, stylesheet: &mut Stylesheet, offset: usize) -> Vec<Declaration> {
    let mut declarations = Vec::new();
    for statement in split_top_level(input, ';') {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        let Some(colon) = top_level_position(statement, ':') else {
            stylesheet.errors.push(ParseError {
                message: "declaration is missing `:`".into(),
                offset,
            });
            continue;
        };
        let property = statement[..colon].trim();
        let value = statement[colon + 1..].trim();
        if property.is_empty() || value.is_empty() {
            stylesheet.errors.push(ParseError {
                message: "declaration has an empty property or value".into(),
                offset,
            });
            continue;
        }
        declarations.push(Declaration {
            property: property.to_ascii_lowercase(),
            value: value.to_owned(),
        });
    }
    declarations
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut parentheses: usize = 0;
    let mut brackets: usize = 0;
    for (index, character) in input.char_indices() {
        if let Some(open_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => quote = Some(character),
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            character if character == delimiter && parentheses == 0 && brackets == 0 => {
                pieces.push(&input[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    pieces.push(&input[start..]);
    pieces
}

fn top_level_position(input: &str, wanted: char) -> Option<usize> {
    let pieces = split_top_level(input, wanted);
    if pieces.len() > 1 {
        Some(pieces[0].len())
    } else {
        None
    }
}

/// Remove comments without damaging `/* ... */` text inside a quoted string.
fn remove_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut quote = None;
    let mut escaped = false;
    while index < bytes.len() {
        let character = bytes[index] as char;
        if let Some(open_quote) = quote {
            result.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == open_quote {
                quote = None;
            }
            index += 1;
        } else if character == '"' || character == '\'' {
            quote = Some(character);
            result.push(character);
            index += 1;
        } else if index + 1 < bytes.len() && &bytes[index..index + 2] == b"/*" {
            index += 2;
            while index + 1 < bytes.len() && &bytes[index..index + 2] != b"*/" {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
        } else {
            result.push(character);
            index += 1;
        }
    }
    result
}

pub fn specificity(selector: &Selector) -> Specificity {
    let mut result = Specificity(0, 0, 0);
    let mut input = selector.0.as_str();
    let mut type_position = true;
    while !input.is_empty() {
        if type_position && input.as_bytes()[0].is_ascii_alphabetic() {
            result.2 += 1;
            input = skip_identifier(input);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix('#') {
            result.0 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix('.') {
            result.1 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix("::") {
            result.2 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if let Some(rest) = input.strip_prefix(':') {
            result.1 += 1;
            input = skip_identifier(rest);
            type_position = false;
        } else if input.starts_with('[') {
            result.1 += 1;
            input = input
                .get(input.find(']').unwrap_or(input.len() - 1) + 1..)
                .unwrap_or("");
            type_position = false;
        } else if matches!(
            input.as_bytes()[0] as char,
            ' ' | '\t' | '\n' | '\r' | '>' | '+' | '~'
        ) {
            input = &input[1..];
            type_position = true;
        } else {
            input = &input[1..];
            type_position = false;
        }
    }
    result
}

fn skip_identifier(input: &str) -> &str {
    let end = input
        .find(|character: char| {
            !character.is_ascii_alphanumeric() && character != '_' && character != '-'
        })
        .unwrap_or(input.len());
    &input[end..]
}

/// Cascade precedence for a declaration, compared lexicographically.
///
/// `level` orders origins and importance per CSS Cascade 4 §6.1: normal
/// user-agent < normal author < normal inline style < important author <
/// important inline style < important user-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CascadePriority {
    level: u8,
    specificity: Specificity,
    order: usize,
}

/// Applies the bundled user-agent stylesheet, author stylesheets, inline
/// styles, and inheritance to an HTML document.
///
/// Declarations compete by origin and importance (`!important` is honored and
/// stripped from the value), then specificity, then source order. Shorthands
/// are expanded to longhands before they compete; see [`shorthands::expand`].
/// The CSS-wide keywords `inherit`, `initial`, and `unset` are resolved;
/// `revert` and `revert-layer` are treated as `unset`.
///
/// Inherited properties receive the parent's computed value. Every element
/// gets a computed `font-size` in absolute CSS pixels (CSS Fonts 4 §2.5), so
/// descendants inherit an absolute size rather than re-resolving `em`. Other
/// values are passed through as specified text.
pub fn style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles {
    let mut computed: Vec<(NodeID, Vec<Declaration>)> = Vec::new();
    let mut root_font_size = None;
    for (index, node) in dom.nodes.iter().enumerate() {
        let NodeKind::Element(element) = &node.kind else {
            continue;
        };
        let parent = node
            .parent
            .and_then(|parent| computed_style(&computed, parent));
        let parent_value = |property: &str| {
            parent.and_then(|declarations| {
                declarations
                    .iter()
                    .find(|declaration| declaration.property == property)
                    .map(|declaration| declaration.value.clone())
            })
        };

        let mut cascaded = BTreeMap::<String, (CascadePriority, String)>::new();
        let mut apply = |declaration: &Declaration,
                         normal_level: u8,
                         important_level: u8,
                         specificity,
                         order| {
            let (value, important) = split_important(&declaration.value);
            let level = if important {
                important_level
            } else {
                normal_level
            };
            let priority = CascadePriority {
                level,
                specificity,
                order,
            };
            let expanded = shorthands::expand(&Declaration {
                property: declaration.property.clone(),
                value: value.to_owned(),
            });
            for longhand in expanded {
                let wins = cascaded
                    .get(&longhand.property)
                    .is_none_or(|(current, _)| priority >= *current);
                if wins {
                    cascaded.insert(longhand.property, (priority, longhand.value));
                }
            }
        };
        for (sheet_index, (normal_level, important_level, sheet)) in
            std::iter::once((1, 6, user_agent_stylesheet()))
                .chain(stylesheets.iter().map(|sheet| (2, 4, sheet)))
                .enumerate()
        {
            for (rule_index, rule) in sheet.rules.iter().enumerate() {
                if !matches_selector(dom, NodeID::new(index), &rule.selector.0) {
                    continue;
                }
                let order = sheet_index
                    .saturating_mul(1_000_000)
                    .saturating_add(rule_index);
                let selector_specificity = specificity(&rule.selector);
                for declaration in &rule.declarations {
                    apply(
                        declaration,
                        normal_level,
                        important_level,
                        selector_specificity,
                        order,
                    );
                }
            }
        }
        if let Some(style) = attribute_value(element, "style") {
            let mut inline_errors = Stylesheet::default();
            for (position, declaration) in parse_declarations(style, &mut inline_errors, 0)
                .iter()
                .enumerate()
            {
                apply(declaration, 3, 5, Specificity(0, 0, 0), position);
            }
        }

        let mut properties = BTreeMap::<String, String>::new();
        if let Some(parent) = parent {
            for declaration in parent {
                if is_inherited_property(&declaration.property) {
                    properties.insert(declaration.property.clone(), declaration.value.clone());
                }
            }
        }
        properties
            .entry("color".to_owned())
            .or_insert_with(initial_color_value);
        for (property, (_, value)) in cascaded {
            let keyword = value.trim().to_ascii_lowercase();
            let inherit = match keyword.as_str() {
                "inherit" => true,
                "unset" | "revert" | "revert-layer" => is_inherited_property(&property),
                "initial" => false,
                _ => {
                    properties.insert(property, value);
                    continue;
                }
            };
            match parent_value(&property).filter(|_| inherit) {
                Some(inherited) => {
                    properties.insert(property, inherited);
                }
                None if property == "color" => {
                    properties.insert(property, initial_color_value());
                }
                None => {
                    properties.remove(&property);
                }
            }
        }

        let parent_font_size = parent_value("font-size")
            .and_then(|value| {
                values::compute_font_size(
                    &value,
                    values::MEDIUM_FONT_SIZE,
                    values::MEDIUM_FONT_SIZE,
                )
            })
            .unwrap_or(values::MEDIUM_FONT_SIZE);
        let root_size = root_font_size.unwrap_or(values::MEDIUM_FONT_SIZE);
        let font_size = properties
            .get("font-size")
            .and_then(|value| values::compute_font_size(value, parent_font_size, root_size))
            .unwrap_or(parent_font_size);
        properties.insert("font-size".to_owned(), values::format_px(font_size));
        if root_font_size.is_none() {
            root_font_size = Some(font_size);
        }
        computed.push((
            NodeID::new(index),
            properties
                .into_iter()
                .map(|(property, value)| Declaration { property, value })
                .collect(),
        ));
    }
    ComputedStyles { styles: computed }
}

/// Splits a trailing `!important` annotation from a declaration value.
fn split_important(value: &str) -> (&str, bool) {
    let trimmed = value.trim_end();
    let lower = trimmed.to_ascii_lowercase();
    if let Some(before) = lower.strip_suffix("important") {
        let before = before.trim_end();
        if let Some(rest) = before.strip_suffix('!') {
            return (trimmed[..rest.len()].trim_end(), true);
        }
    }
    (trimmed, false)
}

/// The initial `color`, taken from the user-agent `html` rule.
fn initial_color_value() -> String {
    user_agent_stylesheet()
        .rules
        .iter()
        .find(|rule| rule.selector.0 == "html")
        .and_then(|rule| {
            rule.declarations
                .iter()
                .find(|item| item.property == "color")
        })
        .expect("user-agent stylesheet must define html color")
        .value
        .clone()
}

fn computed_style(
    styles: &[(NodeID, Vec<Declaration>)],
    parent: NodeID,
) -> Option<&Vec<Declaration>> {
    styles
        .binary_search_by_key(&parent.get(), |(id, _)| id.get())
        .ok()
        .map(|position| &styles[position].1)
}

fn is_inherited_property(property: &str) -> bool {
    matches!(
        property,
        "color"
            | "font-family"
            | "font-size"
            | "font-style"
            | "font-weight"
            | "line-height"
            | "text-align"
            | "visibility"
    )
}

fn attribute_value<'a>(element: &'a html::ElementData, name: &str) -> Option<&'a str> {
    element
        .get_attributes()
        .iter()
        .find(|attribute| attribute.get_name() == name)
        .map(|attribute| attribute.get_value().as_str())
}

fn matches_selector(dom: &HTMLDocument, node_id: NodeID, selector: &str) -> bool {
    let parts = selector.split_whitespace().collect::<Vec<_>>();
    let Some(last) = parts.last() else {
        return false;
    };
    if !matches_compound(dom, node_id, last) {
        return false;
    }
    let mut current = node_id;
    for part in parts.iter().rev().skip(1) {
        let mut ancestor = dom.nodes[current.get()].parent;
        let mut found = None;
        while let Some(candidate) = ancestor {
            if matches_compound(dom, candidate, part) {
                found = Some(candidate);
                break;
            }
            ancestor = dom.nodes[candidate.get()].parent;
        }
        let Some(found) = found else { return false };
        current = found;
    }
    true
}

fn matches_compound(dom: &HTMLDocument, node_id: NodeID, selector: &str) -> bool {
    let Some(node) = dom.node(node_id) else {
        return false;
    };
    let NodeKind::Element(element) = &node.kind else {
        return false;
    };
    let mut index = 0;
    if selector
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic())
    {
        let end = selector
            .find(['#', '.', ':', '['])
            .unwrap_or(selector.len());
        if element.get_name() != &selector[..end].to_ascii_lowercase() {
            return false;
        }
        index = end;
    }
    while index < selector.len() {
        let marker = selector.as_bytes()[index] as char;
        if marker == '#' || marker == '.' {
            let start = index + 1;
            let end = selector[start..]
                .find(['#', '.', ':', '['])
                .map_or(selector.len(), |offset| start + offset);
            let value = &selector[start..end];
            let matches = if marker == '#' {
                attribute_value(element, "id") == Some(value)
            } else {
                attribute_value(element, "class")
                    .is_some_and(|classes| classes.split_whitespace().any(|class| class == value))
            };
            if !matches {
                return false;
            }
            index = end;
        } else {
            return false;
        }
    }
    true
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
        assert_eq!(sheet.rules[0].declarations.len(), 3);
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parses_at_rules_and_nested_rules() {
        let sheet = parse_stylesheet(
            "@media screen and (min-width: 10px) { body { color: red; } } @import url('x.css');",
        );
        assert_eq!(sheet.at_rules.len(), 2);
        assert_eq!(sheet.at_rules[0].name, "media");
        assert_eq!(sheet.at_rules[0].rules.len(), 1);
        assert_eq!(sheet.at_rules[1].prelude, "url('x.css')");
    }

    #[test]
    fn recovers_from_bad_declaration() {
        let (sheet, errors) =
            parse_stylesheet_with_errors("p { color: red; broken; background: blue; }");
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn specificity_orders_ids_classes_and_elements() {
        assert!(specificity(&Selector("#main".into())) > specificity(&Selector(".card".into())));
        assert!(specificity(&Selector(".card".into())) > specificity(&Selector("div".into())));
        assert_eq!(
            specificity(&Selector("div.card#main".into())),
            Specificity(1, 1, 1)
        );
    }

    #[test]
    fn cascades_matches_and_inlines() {
        let document = html::parse_raw_html(
            "<div><p id='target' class='intro' style='color: black'>Hello</p></div>".into(),
        );
        let stylesheet =
            parse_stylesheet("p { color: green; } .intro { color: red; font-size: 20px; }");
        let styles = style_document(&document, &[stylesheet]);
        let paragraph = document
            .get_element_by_id("target")
            .expect("paragraph node id");
        let declarations = styles
            .styles
            .iter()
            .find(|(id, _)| id.get() == paragraph.get())
            .expect("computed paragraph");
        assert!(declarations.1.contains(&Declaration {
            property: "color".into(),
            value: "black".into()
        }));
        assert!(declarations.1.contains(&Declaration {
            property: "font-size".into(),
            value: "20px".into()
        }));
    }

    #[test]
    fn parses_common_color_forms_and_rejects_unsupported_values() {
        assert_eq!(
            parse_color("red"),
            Some(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
        assert_eq!(
            parse_color("#abc"),
            Some(CssColor {
                r: 170,
                g: 187,
                b: 204,
                a: 255
            })
        );
        assert_eq!(
            parse_color("rgb(18, 52, 86)"),
            Some(CssColor {
                r: 18,
                g: 52,
                b: 86,
                a: 255
            })
        );
        assert_eq!(parse_color("transparent"), None);
        assert_eq!(parse_color("#ééé"), None);
        assert_eq!(parse_color("rgb(256, 0, 0)"), None);
    }

    #[test]
    fn bundled_defaults_cascade_before_author_styles_and_inherit() {
        let document = html::parse_raw_html(
            "<html id='root'><body><main id='content'>Hello</main></body></html>".into(),
        );
        let html = document.get_element_by_id("root").expect("html node");
        let main = document.get_element_by_id("content").expect("main node");
        let defaults = style_document(&document, &[]);
        assert_eq!(defaults.color_for(html, "background-color"), None);
        assert_eq!(defaults.color_for(main, "color"), parse_color("black"));

        let author = parse_stylesheet("html { color: red; background-color: #123456; }");
        let styled = style_document(&document, &[author]);
        assert_eq!(
            styled.color_for(html, "background-color"),
            parse_color("#123456")
        );
        assert_eq!(styled.color_for(main, "color"), parse_color("red"));
    }

    fn computed(
        styles: &ComputedStyles,
        document: &HTMLDocument,
        id: &str,
        property: &str,
    ) -> Option<String> {
        let node = document.get_element_by_id(id).expect("element with id");
        styles.value_for(node, property).map(str::to_owned)
    }

    #[test]
    fn important_declarations_beat_later_and_more_specific_ones() {
        let document = html::parse_raw_html("<p id='target' class='a'>Hi</p>".into());
        let sheet = parse_stylesheet(
            "p { color: green !important; } .a { color: red; } #target { color: blue; }",
        );
        let styles = style_document(&document, &[sheet]);
        assert_eq!(
            computed(&styles, &document, "target", "color").as_deref(),
            Some("green")
        );
    }

    #[test]
    fn author_shorthand_overrides_user_agent_longhand() {
        let document = html::parse_raw_html("<p id='target'>Hi</p>".into());
        let styles = style_document(&document, &[parse_stylesheet("p { margin: 0 }")]);
        assert_eq!(
            computed(&styles, &document, "target", "margin-top").as_deref(),
            Some("0")
        );
        assert_eq!(
            computed(&styles, &document, "target", "margin-left").as_deref(),
            Some("0")
        );
    }

    #[test]
    fn font_size_inherits_as_an_absolute_length() {
        let document = html::parse_raw_html(
            "<div id='outer' style='font-size: 2em'><span id='inner'>Hi</span><b id='rel' style='font-size: 50%'>x</b></div>"
                .into(),
        );
        let styles = style_document(&document, &[]);
        assert_eq!(
            computed(&styles, &document, "outer", "font-size").as_deref(),
            Some("32px")
        );
        assert_eq!(
            computed(&styles, &document, "inner", "font-size").as_deref(),
            Some("32px")
        );
        assert_eq!(
            computed(&styles, &document, "rel", "font-size").as_deref(),
            Some("16px")
        );
    }

    #[test]
    fn css_wide_keywords_resolve_against_the_parent() {
        let document = html::parse_raw_html(
            "<div id='parent' style='color: red; margin-left: 5px'><p id='child' style='color: initial; margin-left: inherit'>Hi</p></div>"
                .into(),
        );
        let styles = style_document(&document, &[]);
        assert_eq!(
            computed(&styles, &document, "child", "color").as_deref(),
            Some("black")
        );
        assert_eq!(
            computed(&styles, &document, "child", "margin-left").as_deref(),
            Some("5px")
        );
    }
}
