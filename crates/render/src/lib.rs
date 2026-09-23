//! Software rasterization and compositor primitives.

#![forbid(unsafe_code)]

mod compositor;
mod display_item;
mod fonts;
mod frame;

pub use compositor::Compositor;
pub use display_item::DisplayItem;
pub use frame::Frame;
