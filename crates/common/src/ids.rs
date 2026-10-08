//! Strongly typed identifiers shared across crates.
//!
//! Prefer newtype wrappers over raw integers/strings (see architecture §3.1).

/// Index of a node in an arena-allocated DOM.
///
/// The HTML team owns the DOM representation; this newtype only fixes the
/// *shape* of the handle other crates pass around, not what it points at.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape as the
// owning teams need; changes here require cross-team review (architecture §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    /// Wraps a raw arena index.
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the raw arena index.
    pub const fn index(self) -> u32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_index() {
        assert_eq!(NodeId::new(7).index(), 7);
    }

    #[test]
    fn equal_indices_are_equal_ids() {
        assert_eq!(NodeId::new(3), NodeId::new(3));
        assert_ne!(NodeId::new(3), NodeId::new(4));
    }

    #[test]
    fn orders_by_index() {
        assert!(NodeId::new(1) < NodeId::new(2));
    }
}
