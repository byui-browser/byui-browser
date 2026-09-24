//! The layout-facing projection of the document tree.

use common::ids::NodeId;
use html::{HtmlDocument, NodeKind};

/// The layout-facing projection of an HTML document.
///
/// Until the CSS stage exists, this contains one shared arena index for each
/// HTML element and no computed style information.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StyledDom {
    /// HTML arena indices represented as layout nodes.
    pub nodes: Vec<NodeId>,
}

impl StyledDom {
    /// Creates the first layout slice's projection: every HTML element is a
    /// visible block. CSS will eventually provide the style information.
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

/// Converts a layout node ID back to the corresponding HTML arena ID.
///
/// This is valid because [`StyledDom::from_html_document`] preserves the HTML
/// document's arena indices. Callers must not use it with IDs from another
/// arena.
pub fn html_node_id(node_id: NodeId) -> html::NodeId {
    html::NodeId(node_id.index() as usize)
}
