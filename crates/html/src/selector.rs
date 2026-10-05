use common::ids::NodeId as CommonNodeId;
use std::collections::BTreeMap;

use crate::{ElementData, HTMLDocument, HTMLElement, NodeKind};

/// A selector parsing error returned by [`HTMLDocument::query_selector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorError {
    /// A description of the invalid selector.
    pub message: String,
}

impl SelectorError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SelectorError {}

pub trait Query {
    fn query(&self, selector: &str) -> Vec<HTMLElement>;

    #[allow(non_snake_case)]
    fn Query(&self, selector: String) -> Vec<HTMLElement> {
        self.query(&selector)
    }
}

impl Query for HTMLDocument {
    fn query(&self, selector: &str) -> Vec<HTMLElement> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let NodeKind::Element(element) = &node.kind else {
                    return None;
                };
                matches_selector(element, selector)
                    .then(|| project_element(self, CommonNodeId::new(index as u32), index, element))
            })
            .collect()
    }
}

impl HTMLDocument {
    /// Returns the first descendant matching `selector` in tree order.
    ///
    /// This follows the DOM `ParentNode.querySelector()` contract: the
    /// document itself is not a candidate, no detached nodes are searched,
    /// and an invalid selector is reported as a [`SelectorError`] (the Rust
    /// equivalent of the DOM `SyntaxError` exception).
    ///
    /// The currently supported selector forms are element names, `*`, IDs,
    /// classes, and combinations of those simple selectors (for example
    /// `div.card#main`). Selector lists separated by commas are supported.
    pub fn query_selector(&self, selector: &str) -> Result<Option<crate::NodeId>, SelectorError> {
        let selectors = parse_selector_list(selector)?;

        fn find(
            document: &HTMLDocument,
            parent: crate::NodeId,
            selectors: &[SimpleSelector],
        ) -> Option<crate::NodeId> {
            let children = document.node(parent)?.children.clone();
            for child in children {
                let node = document.node(child)?;
                if let NodeKind::Element(element) = &node.kind
                    && selectors.iter().any(|selector| selector.matches(element))
                {
                    return Some(child);
                }
                if let Some(found) = find(document, child, selectors) {
                    return Some(found);
                }
            }
            None
        }

        Ok(find(self, self.root, &selectors))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleSelector {
    element_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl SimpleSelector {
    fn matches(&self, element: &ElementData) -> bool {
        if let Some(name) = &self.element_name
            && element.name != *name
        {
            return false;
        }
        if let Some(id) = &self.id
            && !element
                .attributes
                .iter()
                .any(|attribute| attribute.name == "id" && attribute.value == *id)
        {
            return false;
        }
        self.classes.iter().all(|class| {
            element.attributes.iter().any(|attribute| {
                attribute.name == "class"
                    && attribute
                        .value
                        .split_whitespace()
                        .any(|value| value == class)
            })
        })
    }
}

fn parse_selector_list(selector: &str) -> Result<Vec<SimpleSelector>, SelectorError> {
    let selectors = selector
        .split(',')
        .map(str::trim)
        .map(parse_simple_selector)
        .collect::<Result<Vec<_>, _>>()?;
    if selectors.is_empty() {
        return Err(SelectorError::new("selector must not be empty"));
    }
    Ok(selectors)
}

fn parse_simple_selector(selector: &str) -> Result<SimpleSelector, SelectorError> {
    if selector.is_empty() || selector.chars().any(char::is_whitespace) {
        return Err(SelectorError::new("unsupported or empty selector"));
    }

    let mut parsed = SimpleSelector {
        element_name: None,
        id: None,
        classes: Vec::new(),
    };
    let mut remaining = selector;
    if let Some(first) = remaining.chars().next()
        && first != '#'
        && first != '.'
        && first != '*'
    {
        let end = remaining.find(['#', '.']).unwrap_or(remaining.len());
        let name = &remaining[..end];
        if !is_identifier(name) {
            return Err(SelectorError::new("invalid element selector"));
        }
        parsed.element_name = Some(name.to_ascii_lowercase());
        remaining = &remaining[end..];
    } else if remaining.starts_with('*') {
        remaining = &remaining[1..];
    }

    while !remaining.is_empty() {
        let marker = remaining.as_bytes()[0] as char;
        if marker != '#' && marker != '.' {
            return Err(SelectorError::new("invalid selector syntax"));
        }
        let value_start = 1;
        let value_end = remaining[1..]
            .find(['#', '.'])
            .map_or(remaining.len(), |index| index + value_start);
        let value = &remaining[value_start..value_end];
        if !is_identifier(value) {
            return Err(SelectorError::new("invalid selector name"));
        }
        if marker == '#' {
            if parsed.id.replace(value.to_owned()).is_some() {
                return Err(SelectorError::new("selector has multiple IDs"));
            }
        } else {
            parsed.classes.push(value.to_owned());
        }
        remaining = &remaining[value_end..];
    }

    if parsed.element_name.is_none() && parsed.id.is_none() && parsed.classes.is_empty() {
        return Err(SelectorError::new(
            "selector must contain a simple selector",
        ));
    }
    Ok(parsed)
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
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

fn matches_selector(element: &ElementData, selector: &str) -> bool {
    match selector.strip_prefix('#') {
        Some(id) => element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "id" && attribute.value == id),
        None => match selector.strip_prefix('.') {
            Some(class) => element.attributes.iter().any(|attribute| {
                attribute.name == "class"
                    && attribute
                        .value
                        .split_whitespace()
                        .any(|value| value == class)
            }),
            None => element.name == selector.to_ascii_lowercase(),
        },
    }
}
