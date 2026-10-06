//! CSS parsing, cascade, inheritance, and computed styles.
//!
//! The parser is intentionally forgiving, like a browser parser: it keeps
//! valid rules when a neighboring rule is malformed and reports malformed
//! pieces as [`ParseError`] values.

#![forbid(unsafe_code)]

use common::ids::NodeId;
use html::{HtmlDocument, NodeKind};
use std::collections::BTreeMap;

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
pub type Dom = HtmlDocument;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    pub styles: Vec<(NodeId, Vec<Declaration>)>,
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

/// Applies author styles, source order, inline styles, and basic inheritance.
pub fn style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles {
    let mut computed = Vec::new();
    for (index, node) in dom.nodes.iter().enumerate() {
        let NodeKind::Element(element) = &node.kind else {
            continue;
        };
        let mut values = BTreeMap::<String, (Specificity, usize, String)>::new();
        if let Some(parent) = node
            .parent
            .and_then(|parent| computed_style(&computed, parent))
        {
            for declaration in parent {
                if is_inherited_property(&declaration.property) {
                    values.insert(
                        declaration.property.clone(),
                        (Specificity(0, 0, 0), 0, declaration.value.clone()),
                    );
                }
            }
        }
        for (sheet_index, sheet) in stylesheets.iter().enumerate() {
            for (rule_index, rule) in sheet.rules.iter().enumerate() {
                if !matches_selector(dom, html::NodeId(index), &rule.selector.0) {
                    continue;
                }
                let rank = sheet_index
                    .saturating_mul(1_000_000)
                    .saturating_add(rule_index);
                let selector_specificity = specificity(&rule.selector);
                for declaration in &rule.declarations {
                    let replace = values.get(&declaration.property).is_none_or(|current| {
                        (selector_specificity, rank) >= (current.0, current.1)
                    });
                    if replace {
                        values.insert(
                            declaration.property.clone(),
                            (selector_specificity, rank, declaration.value.clone()),
                        );
                    }
                }
            }
        }
        if let Some(style) = attribute_value(element, "style") {
            let mut inline_errors = Stylesheet::default();
            for declaration in parse_declarations(style, &mut inline_errors, 0) {
                values.insert(
                    declaration.property,
                    (
                        Specificity(u32::MAX, u32::MAX, u32::MAX),
                        usize::MAX,
                        declaration.value,
                    ),
                );
            }
        }
        computed.push((
            NodeId::new(index as u32),
            values
                .into_iter()
                .map(|(property, (_, _, value))| Declaration { property, value })
                .collect(),
        ));
    }
    ComputedStyles { styles: computed }
}

fn computed_style<'a>(
    styles: &'a [(NodeId, Vec<Declaration>)],
    parent: html::NodeId,
) -> Option<&'a Vec<Declaration>> {
    styles
        .iter()
        .find(|(id, _)| id.index() as usize == parent.index())
        .map(|(_, declarations)| declarations)
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
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

fn matches_selector(dom: &HtmlDocument, node_id: html::NodeId, selector: &str) -> bool {
    let parts = selector.split_whitespace().collect::<Vec<_>>();
    let Some(last) = parts.last() else {
        return false;
    };
    if !matches_compound(dom, node_id, last) {
        return false;
    }
    let mut current = node_id;
    for part in parts.iter().rev().skip(1) {
        let mut ancestor = dom.nodes[current.index()].parent;
        let mut found = None;
        while let Some(candidate) = ancestor {
            if matches_compound(dom, candidate, part) {
                found = Some(candidate);
                break;
            }
            ancestor = dom.nodes[candidate.index()].parent;
        }
        let Some(found) = found else { return false };
        current = found;
    }
    true
}

fn matches_compound(dom: &HtmlDocument, node_id: html::NodeId, selector: &str) -> bool {
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
        if element.name != selector[..end].to_ascii_lowercase() {
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
    use html::Query;

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
            "<div><p class='intro' style='color: black'>Hello</p></div>".into(),
        );
        let stylesheet =
            parse_stylesheet("p { color: green; } .intro { color: red; font-size: 20px; }");
        let styles = style_document(&document, &[stylesheet]);
        let paragraph = document.query("p").pop().expect("paragraph");
        let declarations = styles
            .styles
            .iter()
            .find(|(id, _)| id.index() == paragraph.id.index())
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
}
