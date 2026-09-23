//! Layout tree data structures.

use common::ids::NodeId;

use crate::Rect;

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutBox {
    pub node: Option<NodeId>,
    pub rect: Rect,
    pub children: Vec<LayoutBox>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree {
    pub root: LayoutBox,
}
