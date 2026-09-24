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
    /// Draws text with the built-in bitmap font.
    Text {
        /// Left edge.
        x: u32,
        /// Top edge.
        y: u32,
        /// Text to draw.
        text: String,
        /// RGBA8 color.
        rgba: [u8; 4],
    },
}
