use common::ids::NodeId as CommonNodeId;
use std::collections::BTreeMap;

use crate::{ElementData, HTMLDocument, HTMLElement, Node, NodeKind};

/// Query an HTML document using the supported CSS selector subset.
pub trait Query {
    /// Returns all matching elements in document order.
    fn query(&self, selector: &str) -> Vec<HTMLElement>;

    /// Returns the first matching element in document order.
    fn query_selector(&self, selector: &str) -> Option<HTMLElement>;

    /// Returns all matching elements in document order.
    fn query_selector_all(&self, selector: &str) -> Vec<HTMLElement>;

    /// Compatibility wrapper for the original query API.
    #[allow(non_snake_case)]
    fn Query(&self, selector: String) -> Vec<HTMLElement> {
        self.query(&selector)
    }
}

impl Query for HTMLDocument {
    fn query(&self, selector: &str) -> Vec<HTMLElement> {
        self.query_selector_all(selector)
    }

    fn query_selector(&self, selector: &str) -> Option<HTMLElement> {
        self.query_selector_all(selector).into_iter().next()
    }

    fn query_selector_all(&self, selector: &str) -> Vec<HTMLElement> {
        query_within(self, self.root, true, selector)
    }
}

pub(crate) fn query_from_node(
    document: &HTMLDocument,
    node: &Node,
    selector: &str,
) -> Vec<HTMLElement> {
    let Some(root) = document
        .nodes
        .iter()
        .position(|candidate| std::ptr::eq(candidate, node))
    else {
        return Vec::new();
    };
    query_within(document, crate::NodeId(root), false, selector)
}

fn query_within(
    document: &HTMLDocument,
    root: crate::NodeId,
    include_root: bool,
    selector: &str,
) -> Vec<HTMLElement> {
    let Some(groups) = parse_selector_list(selector) else {
        return Vec::new();
    };
    let mut matches = Vec::new();
    let Some(root_node) = document.nodes.get(root.0) else {
        return matches;
    };
    let mut pending = if include_root {
        vec![root]
    } else {
        root_node.children.iter().rev().copied().collect()
    };
    while let Some(id) = pending.pop() {
        let Some(node) = document.nodes.get(id.0) else {
            continue;
        };
        if let NodeKind::Element(element) = &node.kind {
            if groups.iter().any(|group| matches_group(document, id.0, group)) {
                matches.push(project_element(
                    document,
                    CommonNodeId::new(id.0 as u32),
                    id.0,
                    element,
                ));
            }
        }
        pending.extend(node.children.iter().rev().copied());
    }
    matches
}

#[derive(Debug)]
struct SelectorGroup {
    compounds: Vec<CompoundSelector>,
    combinators: Vec<Combinator>,
}

#[derive(Debug)]
enum Combinator {
    Descendant,
    Child,
}

#[derive(Debug, Default)]
struct CompoundSelector {
    universal: bool,
    tag: Option<String>,
    ids: Vec<String>,
    classes: Vec<String>,
    attributes: Vec<(String, Option<String>)>,
}

fn parse_selector_list(selector: &str) -> Option<Vec<SelectorGroup>> {
    split_outside_attributes(selector, ',')?
        .into_iter()
        .map(parse_group)
        .collect()
}

fn split_outside_attributes(input: &str, delimiter: char) -> Option<Vec<&str>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quote = None;
    for (index, character) in input.char_indices() {
        match (quote, character) {
            (Some(current), value) if current == value => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') if depth > 0 => quote = Some(character),
            (None, '[') => depth += 1,
            (None, ']') => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            (None, value) if value == delimiter && depth == 0 => {
                parts.push(input[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    if depth != 0 || quote.is_some() {
        return None;
    }
    parts.push(input[start..].trim());
    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    Some(parts)
}

fn parse_group(input: &str) -> Option<SelectorGroup> {
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut buffer = String::new();
    let mut depth = 0;
    let mut quote = None;
    let mut pending_combinator = None;

    for character in input.chars() {
        match (quote, character) {
            (Some(current), value) if current == value => {
                quote = None;
                buffer.push(character);
            }
            (Some(_), _) => buffer.push(character),
            (None, '\'' | '"') if depth > 0 => {
                quote = Some(character);
                buffer.push(character);
            }
            (None, '[') => {
                depth += 1;
                buffer.push(character);
            }
            (None, ']') => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
                buffer.push(character);
            }
            (None, '>') if depth == 0 => {
                if !flush_compound(
                    &mut buffer,
                    &mut compounds,
                    &mut combinators,
                    &mut pending_combinator,
                ) {
                    return None;
                }
                if compounds.is_empty() || matches!(pending_combinator, Some(Combinator::Child)) {
                    return None;
                }
                pending_combinator = Some(Combinator::Child);
            }
            (None, value) if value.is_whitespace() && depth == 0 => {
                if !flush_compound(
                    &mut buffer,
                    &mut compounds,
                    &mut combinators,
                    &mut pending_combinator,
                ) {
                    return None;
                }
                if !compounds.is_empty() && pending_combinator.is_none() {
                    pending_combinator = Some(Combinator::Descendant);
                }
            }
            _ => buffer.push(character),
        }
    }
    if depth != 0
        || quote.is_some()
        || !flush_compound(
            &mut buffer,
            &mut compounds,
            &mut combinators,
            &mut pending_combinator,
        )
    {
        return None;
    }
    if compounds.is_empty() || pending_combinator.is_some() {
        return None;
    }
    if combinators.len() + 1 != compounds.len() {
        return None;
    }
    Some(SelectorGroup {
        compounds,
        combinators,
    })
}

fn flush_compound(
    buffer: &mut String,
    compounds: &mut Vec<CompoundSelector>,
    combinators: &mut Vec<Combinator>,
    pending_combinator: &mut Option<Combinator>,
) -> bool {
    if buffer.trim().is_empty() {
        buffer.clear();
        return true;
    }
    let Some(compound) = parse_compound(buffer.trim()) else {
        return false;
    };
    if !compounds.is_empty() {
        combinators.push(
            pending_combinator
                .take()
                .unwrap_or(Combinator::Descendant),
        );
    }
    compounds.push(compound);
    buffer.clear();
    true
}

pub(crate) fn matches_simple_selector(node: &Node, selector: &str) -> bool {
    let NodeKind::Element(element) = &node.kind else {
        return false;
    };
    parse_compound(selector.trim())
        .is_some_and(|selector| matches_compound(element, &selector))
}

fn parse_compound(input: &str) -> Option<CompoundSelector> {
    let mut selector = CompoundSelector::default();
    let mut position = 0;
    let bytes = input.as_bytes();
    if bytes.first() == Some(&b'*') {
        selector.universal = true;
        position += 1;
    } else if let Some(end) = identifier_end(input, position) {
        selector.tag = Some(input[position..end].to_ascii_lowercase());
        position = end;
    }

    while position < bytes.len() {
        match bytes[position] {
            b'#' | b'.' => {
                let kind = bytes[position];
                position += 1;
                let end = identifier_end(input, position)?;
                let value = input[position..end].to_owned();
                if kind == b'#' {
                    selector.ids.push(value);
                } else {
                    selector.classes.push(value);
                }
                position = end;
            }
            b'[' => {
                let end = find_attribute_end(input, position + 1)?;
                let attribute = input[position + 1..end].trim();
                let (name, value) = match attribute.split_once('=') {
                    Some((name, value)) => {
                        let value = value.trim();
                        let value = if value.len() >= 2
                            && ((value.starts_with('\'') && value.ends_with('\''))
                                || (value.starts_with('"') && value.ends_with('"')))
                        {
                            &value[1..value.len() - 1]
                        } else {
                            value
                        };
                        (name.trim(), Some(value.to_owned()))
                    }
                    None => (attribute, None),
                };
                if name.is_empty() || name.bytes().any(|byte| byte.is_ascii_whitespace()) {
                    return None;
                }
                selector
                    .attributes
                    .push((name.to_ascii_lowercase(), value));
                position = end + 1;
            }
            _ => return None,
        }
    }
    (selector.tag.is_some()
        || !selector.ids.is_empty()
        || !selector.classes.is_empty()
        || !selector.attributes.is_empty()
        || selector.universal)
        .then_some(selector)
}

fn find_attribute_end(input: &str, start: usize) -> Option<usize> {
    let mut quote = None;
    for (offset, character) in input[start..].char_indices() {
        match (quote, character) {
            (Some(current), value) if current == value => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(character),
            (None, ']') => return Some(start + offset),
            _ => {}
        }
    }
    None
}

fn identifier_end(input: &str, start: usize) -> Option<usize> {
    let end = input[start..]
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
        })
        .map(|(offset, character)| start + offset + character.len_utf8())
        .last()?;
    Some(end)
}

fn matches_group(document: &HTMLDocument, index: usize, group: &SelectorGroup) -> bool {
    matches_at(document, index, group, group.compounds.len() - 1)
}

fn matches_at(
    document: &HTMLDocument,
    index: usize,
    group: &SelectorGroup,
    compound_index: usize,
) -> bool {
    let NodeKind::Element(element) = &document.nodes[index].kind else {
        return false;
    };
    if !matches_compound(element, &group.compounds[compound_index]) {
        return false;
    }
    if compound_index == 0 {
        return true;
    }

    match group.combinators[compound_index - 1] {
        Combinator::Child => document.nodes[index]
            .parent
            .is_some_and(|parent| matches_at(document, parent.0, group, compound_index - 1)),
        Combinator::Descendant => {
            let mut parent = document.nodes[index].parent;
            while let Some(parent_id) = parent {
                if matches_at(document, parent_id.0, group, compound_index - 1) {
                    return true;
                }
                parent = document.nodes[parent_id.0].parent;
            }
            false
        }
    }
}

fn matches_compound(element: &ElementData, selector: &CompoundSelector) -> bool {
    if selector
        .tag
        .as_ref()
        .is_some_and(|tag| tag != &element.name)
    {
        return false;
    }
    for id in &selector.ids {
        if !element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == *id)
        {
            return false;
        }
    }
    for class in &selector.classes {
        if !element.attributes.iter().any(|attribute| {
            attribute.name == "class"
                && attribute
                    .value
                    .split_whitespace()
                    .any(|value| value == class)
        }) {
            return false;
        }
    }
    selector.attributes.iter().all(|(name, value)| {
        element.attributes.iter().any(|attribute| {
            attribute.name == *name
                && value
                    .as_ref()
                    .is_none_or(|expected| attribute.value == *expected)
        })
    })
}

fn project_element(
    document: &HTMLDocument,
    id: CommonNodeId,
    index: usize,
    element: &ElementData,
) -> HTMLElement {
    let parent = document.nodes[index]
        .parent
        .map(|parent| CommonNodeId::new(parent.0 as u32));
    let children = document.nodes[index]
        .children
        .iter()
        .filter_map(|child| match document.nodes[child.0].kind {
            NodeKind::Element(_) => Some(CommonNodeId::new(child.0 as u32)),
            _ => None,
        })
        .collect();
    let attributes = element
        .attributes
        .iter()
        .map(|attribute| (attribute.name.clone(), attribute.value.clone()))
        .collect::<BTreeMap<_, _>>();
    HTMLElement {
        id,
        name: element.name.clone(),
        attributes,
        parent,
        children,
        text: document.text_content(crate::NodeId(index)),
    }
}

