use common::ids::NodeId as CommonNodeId;
use std::collections::BTreeMap;

use crate::{ElementData, HTMLDocument, HTMLElement, NodeKind};

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
