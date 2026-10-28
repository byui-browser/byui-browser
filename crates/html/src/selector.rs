use crate::{ElementData, HTMLDocument, Node, NodeKind};

// #[derive(Debug, Clone, PartialEq, Eq)]
// pub struct SelectorError {
//     /// A description of the invalid selector.
//     pub message: String,
// }

// impl SelectorError {
//     fn new(message: impl Into<String>) -> Self {
//         Self {
//             message: message.into(),
//         }
//     }
// }

// impl std::fmt::Display for SelectorError {
//     fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
//         formatter.write_str(&self.message)
//     }
// }

// impl std::error::Error for SelectorError {}

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
            .get_attributes()
            .iter()
            .any(|attribute| attribute.get_name() == "id" && attribute.get_value() == id),
        None => match selector.strip_prefix('.') {
            Some(class) => element.get_attributes().iter().any(|attribute| {
                attribute.get_name() == "class"
                    && attribute
                        .get_value()
                        .split_whitespace()
                        .any(|value| value == class)
            }),
            None => element.get_name().as_str() == selector.to_ascii_lowercase(),
        },
    }
}
