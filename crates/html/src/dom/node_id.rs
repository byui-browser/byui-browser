#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeID {
    id: usize,
}

impl NodeID {
    // Constructs NodeID and sets immutable id.
    pub fn new(set_value: usize) -> Self {
        Self { id: set_value }
    }

    // Gets id.
    pub const fn get(&self) -> usize {
        self.id
    }
}
