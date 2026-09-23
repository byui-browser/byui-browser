//! Box tree construction and layout.

#![forbid(unsafe_code)]

mod geometry;
mod layout;
mod styled_dom;
mod tree;

pub use geometry::{Rect, Size};
pub use layout::layout_tree;
pub use styled_dom::StyledDom;
pub use tree::{LayoutBox, LayoutTree};
