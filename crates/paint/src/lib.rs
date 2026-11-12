//! Display-list generation from layout output.
//!
//! This crate converts CSS-pixel layout boxes and HTML text into ordered paint
//! commands. It sits between `layout` and `render`; it does not rasterize,
//! allocate a frame buffer, or talk to a GPU.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod display_list;
mod painter;
mod types;

pub use display_list::{DisplayItem, DisplayList, build_display_list};
pub use painter::paint_document;
pub use types::{Color, PaintBox, Rect};
