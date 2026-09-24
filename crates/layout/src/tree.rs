//! Layout tree data structures.

use common::ids::NodeId;

use crate::Rect;

/// A laid-out box associated with an optional HTML node.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutBox {
    /// The HTML node represented by this box, or `None` for the viewport root.
    pub node: Option<NodeId>,
    /// The box's CSS-pixel bounds.
    pub rect: Rect,
    /// Nested boxes. The first slice leaves this empty and flattens elements.
    pub children: Vec<LayoutBox>,
}

/// The complete layout output for one document and viewport.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree {
    /// The anonymous viewport box.
    pub root: LayoutBox,
}
