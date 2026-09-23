//! Display list generation from the layout tree.

#![forbid(unsafe_code)]

mod display_list;
mod painter;
mod types;

pub use display_list::{DisplayItem, DisplayList, build_display_list};
pub use painter::{paint_document, paint_layout};
pub use types::{Color, PaintBox, Rect};
