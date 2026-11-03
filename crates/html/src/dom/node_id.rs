#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeID {
    id: usize,
}

impl Default for NodeID {
    fn default() -> Self {
        Self::new(0)
    }
}

impl NodeID {
    // Creates NodeID with immutable id.
    pub fn new(set_value: usize) -> Self {
        Self { id: set_value }
    }

    // Gets id.
    pub const fn get(&self) -> usize {
        self.id
    }
}

// pub type NodeId = NodeID;
