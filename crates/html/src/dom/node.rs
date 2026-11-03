use super::{node_id::NodeID, node_kind::NodeKind, source_span::SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub parent: Option<NodeID>,
    pub children: Vec<NodeID>,
    pub kind: NodeKind,
    pub span: Option<SourceSpan>,
}

impl Default for Node {
    fn default() -> Self {
        Self::new()
    }
}

impl Node {
    pub fn new() -> Self {
        Self {
            parent: Option::None,
            children: vec![],
            kind: NodeKind::None,
            span: Option::None,
        }
    }

    pub fn append_child(_node: Node) {}

    pub fn remove() {}
}
