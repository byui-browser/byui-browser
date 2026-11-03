//! Rasterizer-facing drawing commands.

/// A rasterizer-facing drawing command in device pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    /// Fills a rectangle.
    FillRect {
        /// Left edge.
        x: u32,
        /// Top edge.
        y: u32,
        /// Width.
        width: u32,
        /// Height.
        height: u32,
        /// RGBA8 color.
        rgba: [u8; 4],
    },
    /// Draws text with the built-in [`crate::BitmapFont`].
    Text {
        /// Left edge of the first glyph cell, in device pixels.
        x: f32,
        /// Alphabetic baseline, in device pixels.
        baseline: f32,
        /// Font size in device pixels; glyphs scale by
        /// `font_size / BitmapFont::UNITS_PER_EM`.
        font_size: f32,
        /// Text to draw.
        text: String,
        /// RGBA8 color.
        rgba: [u8; 4],
    },
}
