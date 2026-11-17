use super::{node_id::NodeID, node_kind::NodeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    parent: Option<NodeID>,
    children: Vec<NodeID>,
    kind: NodeKind,
}

impl Default for Node {
    fn default() -> Self {
        Self::new(Option::None, vec![], NodeKind::None)
    }
}

impl Node {
    /// Constructs node
    pub fn new(parent: Option<NodeID>, children: Vec<NodeID>, kind: NodeKind) -> Self {
        Self {
            parent,
            children,
            kind,
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
    // Only adds child id to children if not already listed.
    pub fn add_child(&mut self, child_id: NodeID) {
        if !self
            .children
            .iter()
            .any(|id_ref| id_ref.get() == child_id.get())
        {
            self.children.push(child_id);
        }
    }
    // Only removes child id from children if listed.
    pub fn remove_child(&mut self, child_id: NodeID) {
        if let Some(index) = self
            .children
            .iter()
            .position(|id_ref| id_ref.get() == child_id.get())
        {
            self.children.remove(index);
        }
    }

    /// Gets reference of node kind.
    pub fn get_kind(&self) -> &NodeKind {
        &self.kind
    }
    /// Gets mutable reference of node kind.
    pub fn get_mut_kind(&mut self) -> &mut NodeKind {
        &mut self.kind
    }

    /// Sets node kind.
    pub fn set_kind(&mut self, node_kind: NodeKind) {
        self.kind = node_kind;
    }
}
