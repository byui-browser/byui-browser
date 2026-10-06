//! CSS parsing, cascade, inheritance, and computed styles.

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

/// CSS's view of the HTML team's arena-backed DOM.
pub type Dom = HtmlDocument;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    pub styles: Vec<(NodeId, Vec<Declaration>)>,
}

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let input = remove_comments(input);
    let mut rules = Vec::new();
    for rule_text in input.split('}') {
        let Some((selector_text, declarations_text)) = rule_text.split_once('{') else {
            continue;
        };
        let declarations = parse_declarations(declarations_text);
        for selector in selector_text
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            rules.push(Rule {
                selector: Selector(selector.to_owned()),
                declarations: declarations.clone(),
            });
        }
    }
    Stylesheet { rules }
}

pub fn specificity(selector: &Selector) -> Specificity {
    let mut result = Specificity(0, 0, 0);
    for component in selector.0.split_whitespace() {
        let mut chars = component.chars().peekable();
        if chars.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            result.2 += 1;
        }
        for character in chars {
            match character {
                '#' => result.0 += 1,
                '.' => result.1 += 1,
                _ => {}
            }
        }
    }
    result
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
            for declaration in parse_declarations(style) {
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

fn parse_declarations(input: &str) -> Vec<Declaration> {
    input
        .split(';')
        .filter_map(|declaration| {
            let (property, value) = declaration.split_once(':')?;
            let property = property.trim();
            let value = value.trim();
            (!property.is_empty() && !value.is_empty()).then(|| Declaration {
                property: property.to_owned(),
                value: value.to_owned(),
            })
        })
        .collect()
}

fn remove_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(start) = remaining.find("/*") {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("*/") else {
            return result;
        };
        remaining = &after_start[end + 2..];
    }
    result.push_str(remaining);
    result
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
        let end = selector.find(['#', '.']).unwrap_or(selector.len());
        if element.name != selector[..end].to_ascii_lowercase() {
            return false;
        }
        index = end;
    }
    while index < selector.len() {
        let marker = selector.as_bytes()[index] as char;
        if marker != '#' && marker != '.' {
            return false;
        }
        let start = index + 1;
        let end = selector[start..]
            .find(['#', '.'])
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
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use html::Query;

    #[test]
    fn parses_rules_and_multiple_selectors() {
        let sheet = parse_stylesheet("p, .intro { color: red; background: blue; }");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].declarations[0].property, "color");
    }

    #[test]
    fn specificity_orders_ids_over_classes_over_tags() {
        assert!(specificity(&Selector("#main".into())) > specificity(&Selector(".card".into())));
        assert!(specificity(&Selector(".card".into())) > specificity(&Selector("div".into())));
        assert_eq!(
            specificity(&Selector("div.card#main".into())),
            Specificity(1, 1, 1)
        );
    }

    #[test]
    fn cascades_matches_and_inherits() {
        let document =
            html::parse_raw_html("<div id='main'><p class='intro'>Hello</p></div>".into());
        let stylesheet = parse_stylesheet(
            "div { color: blue; } p { color: green; } .intro { color: red; font-size: 20px; }",
        );
        let styles = style_document(&document, &[stylesheet]);
        let paragraph = document.query("p").pop().expect("paragraph");
        let declarations = styles
            .styles
            .iter()
            .find(|(id, _)| id.index() == paragraph.id.index())
            .expect("computed paragraph");
        assert!(declarations.1.contains(&Declaration {
            property: "color".into(),
            value: "red".into()
        }));
        assert!(declarations.1.contains(&Declaration {
            property: "font-size".into(),
            value: "20px".into()
        }));
    }

    #[test]
    fn inline_style_wins() {
        let document = html::parse_raw_html("<p style='color: red'>Hello</p>".into());
        let styles = style_document(&document, &[parse_stylesheet("p { color: blue; }")]);
        assert!(
            styles
                .styles
                .iter()
                .any(|(_, declarations)| declarations.contains(&Declaration {
                    property: "color".into(),
                    value: "red".into()
                }))
        );
    }
}
