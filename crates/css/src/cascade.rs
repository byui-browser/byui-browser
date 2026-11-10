//! Cascade, inheritance, and the currently supported selector matching.

use crate::model::{ComputedStyles, Declaration, Dom, Specificity, Stylesheet};
use crate::parser::parse_declarations;
use crate::specificity::specificity;
use common::ids::NodeId;
use html::{HTMLDocument, NodeKind};
use std::collections::BTreeMap;

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

fn computed_style(
    styles: &[(NodeId, Vec<Declaration>)],
    parent: html::NodeId,
) -> Option<&Vec<Declaration>> {
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

fn matches_selector(dom: &HTMLDocument, node_id: html::NodeId, selector: &str) -> bool {
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

fn matches_compound(dom: &HTMLDocument, node_id: html::NodeId, selector: &str) -> bool {
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
