//! Box tree construction and layout.
//!
//! This crate converts the HTML/CSS-facing document projection into layout
//! boxes in CSS pixels. It sits between [`html`] and `paint`; it does not
//! rasterize pixels or talk to a GPU.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod geometry;
mod layout;
mod styled_dom;
mod tree;

pub use geometry::{Rect, Size};
pub use layout::layout_tree;
pub use styled_dom::{StyledDom, html_node_id};
pub use tree::{LayoutBox, LayoutTree};
