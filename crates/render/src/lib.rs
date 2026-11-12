//! Software rasterization and compositor primitives.
//!
//! This crate consumes paint commands and produces an RGBA8 frame in device
//! pixels. It is the final stage of this slice; it does not perform layout or
//! HTML parsing. Text is drawn with an [`OutlineFont`] loaded from a font
//! file, or with a small built-in [`BitmapFont`] when no font file is
//! available. Rasterizing uses a software buffer rather than a GPU.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod compositor;
mod display_item;
mod fonts;
mod frame;

pub use compositor::Compositor;
pub use display_item::DisplayItem;
pub use fonts::{BitmapFont, OutlineFont};
pub use frame::Frame;
