//! The layout-facing projection of the document tree.

use common::ids::NodeId;
use html::{HtmlDocument, NodeKind};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StyledDom {
    pub nodes: Vec<NodeId>,
}

impl StyledDom {
    /// Creates the first layout slice's projection: every HTML element is a
    /// visible block. CSS will eventually provide this type.
    pub fn from_html_document(document: &HtmlDocument) -> Self {
        let nodes = document
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                matches!(node.kind, NodeKind::Element(_)).then_some(NodeId::new(index as u32))
            })
            .collect();
        Self { nodes }
    }
}
