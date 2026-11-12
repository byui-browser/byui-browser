use super::{node_id::NodeID, node_kind::NodeKind, source_span::SourceSpan};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    parent: Option<NodeID>,
    children: Vec<NodeID>,
    kind: NodeKind,
    span: Option<SourceSpan>,
}

impl Default for Node {
    fn default() -> Self {
        Self::new(Option::None, vec![], NodeKind::None, Option::None)
    }
}

impl Node {
    /// Constructs node
    pub fn new(
        parent: Option<NodeID>,
        children: Vec<NodeID>,
        kind: NodeKind,
        span: Option<SourceSpan>,
    ) -> Self {
        Self {
            parent,
            children,
            kind,
            span,
        }
    }

    /// Gets a copy of the parent's id.
    pub fn get_parent(&self) -> Option<NodeID> {
        self.parent
    }
    /// Only sets stored parent id.
    pub fn set_parent(&mut self, parent_id: Option<NodeID>) {
        self.parent = parent_id;
    }

    /// Gets a reference of the children ids.
    pub fn get_children(&self) -> &Vec<NodeID> {
        &self.children
    }
    /// Only sets stored children ids.
    pub fn set_children(&mut self, children_ids: Vec<NodeID>) {
        self.children = children_ids;
    }

    /// Gets reference of node kind.
    pub fn get_kind(&self) -> &NodeKind {
        &self.kind
    }
    /// Sets node kind.
    pub fn set_kind(&mut self, node_kind: NodeKind) {
        self.kind = node_kind;
    }

    /// Gets reference of span.
    pub fn get_span(&self) -> &Option<SourceSpan> {
        &self.span
    }
    /// Sets span.
    pub fn set_span(&mut self, span: Option<SourceSpan>) {
        self.span = span;
    }
}
