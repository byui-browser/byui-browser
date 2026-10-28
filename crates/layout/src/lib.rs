//! Box tree construction and layout.
//!
//! This crate turns an HTML document and its computed CSS styles into a
//! fragment tree in CSS pixels: block boxes, line boxes, and text runs with
//! absolute positions. It sits between `css` and `paint`; it does not
//! rasterize pixels or talk to a GPU. Text is measured through the
//! [`FontMetrics`] trait so that layout and the renderer agree on glyph sizes.
//!
//! Start with [`layout_document`]; see its documentation for what is and is
//! not implemented.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod block;
mod box_tree;
mod fonts;
mod geometry;
mod inline;
mod layout;
mod rectangle;
mod style;
mod tree;

pub use fonts::{FontMetrics, VerticalMetrics};
pub use geometry::{Edges, Rect, Sides, Size};
pub use layout::layout_document;
pub use rectangle::create_rectangle;
pub use tree::{
    BoxKind, BoxStyle, InlineBoxFragment, LayoutBox, LayoutTree, LineBox, LineFragment, PaintItem,
    TextFragment,
};

#[cfg(test)]
mod tests;
