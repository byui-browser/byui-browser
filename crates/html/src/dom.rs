use common::ids::NodeId;
use std::collections::BTreeMap;

/// One token produced by the tokenizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// `<name>`
    StartTag(String),
    /// `</name>`
    EndTag(String),
    /// Character data between tags.
    Text(String),
}

/// A single DOM node used by the legacy [`crate::parse`] API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Arena handle for this node.
    pub id: NodeId,
    /// Tag name for elements, `"#text"` for text nodes.
    pub name: String,
    /// Parent handle, `None` for the document root.
    pub parent: Option<NodeId>,
}

/// The document tree produced by [`crate::parse`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dom {
    /// Nodes in arena order; `NodeId` indexes into this vector.
    pub nodes: Vec<Node>,
}

/// An element in an [`HTMLDocument`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTMLElement {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    /// `NodeId::new(u32::MAX)` means that this element has no parent.
    pub parent: NodeId,
    pub children: Vec<NodeId>,
}

/// The location associated with an [`HTMLDocument`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub url: String,
}

/// A parsed HTML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HTMLDocument {
    pub doc_type: String,
    pub nodes: Vec<NodeId>,
    pub location: Location,
    pub(crate) elements: Vec<HTMLElement>,
}
