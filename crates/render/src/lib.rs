//! Software rasterization and compositor primitives.
//!
//! This crate consumes paint commands and produces an RGBA8 frame in device
//! pixels. It is the final stage of this slice; it does not perform layout or
//! HTML parsing. The current implementation uses a small built-in bitmap font
//! and a software buffer rather than a GPU.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod compositor;
mod display_item;
mod fonts;
mod frame;

pub use compositor::Compositor;
pub use display_item::DisplayItem;
pub use frame::Frame;
