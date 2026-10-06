use crate::{ElementData, HTMLDocument, Node, NodeKind};

pub trait Query {
    fn query(&self, selector: &str) -> Vec<Node>;

    #[allow(non_snake_case)]
    fn Query(&self, selector: String) -> Vec<Node> {
        self.query(&selector)
    }
}

impl Query for HTMLDocument {
    fn query(&self, selector: &str) -> Vec<Node> {
        self.nodes
            .iter()
            .filter(|node| match &node.kind {
                NodeKind::Element(element) => matches_selector(element, selector),
                _ => false,
            })
            .cloned()
            .collect()
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
