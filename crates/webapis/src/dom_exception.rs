//! DOM exceptions raised by document tree mutations.

/// A DOM exception returned by an invalid `Document::append_child` operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomException {
    /// The insertion would violate the document's node hierarchy constraints.
    HierarchyRequestError,
    /// The node ID does not refer to a node in this document's arena.
    NotFoundError,
}

impl std::fmt::Display for DomException {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::HierarchyRequestError => "HierarchyRequestError",
            Self::NotFoundError => "NotFoundError",
        };
        formatter.write_str(name)
    }
}

impl std::error::Error for DomException {}
