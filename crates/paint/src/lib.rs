//! Display-list generation from layout output.
//!
//! This crate converts the `layout` fragment tree into ordered paint commands
//! in CSS pixels. It sits between `layout` and `render`; it does not
//! rasterize, allocate a frame buffer, or talk to a GPU.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod display_list;
mod painter;
mod types;

pub use display_list::{DisplayItem, DisplayList, build_display_list};
pub use painter::{CANVAS_COLOR, paint_document};
pub use types::{Color, PaintBox, Rect};
