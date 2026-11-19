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
    /// Fills a rectangle with elliptical corners, anti-aliasing the curves.
    ///
    /// Edges are in device pixels and should lie on pixel boundaries; the
    /// straight edges are not anti-aliased.
    FillRoundedRect {
        /// Left edge.
        left: f32,
        /// Top edge.
        top: f32,
        /// Right edge.
        right: f32,
        /// Bottom edge.
        bottom: f32,
        /// Corner radii in device pixels, `[horizontal, vertical]` for the
        /// top-left, top-right, bottom-right, and bottom-left corners. They
        /// must already fit the rectangle.
        radii: [[f32; 2]; 4],
        /// RGBA8 color.
        rgba: [u8; 4],
    },
    /// Fills a rounded rectangle whose edges fade out with a Gaussian blur:
    /// one outer `box-shadow`.
    ///
    /// Edges are in device pixels and need not lie on pixel boundaries.
    /// Paint reaches about `3 * sigma` pixels past the edges.
    BoxShadow {
        /// Left edge of the shape before blurring.
        left: f32,
        /// Top edge of the shape before blurring.
        top: f32,
        /// Right edge of the shape before blurring.
        right: f32,
        /// Bottom edge of the shape before blurring.
        bottom: f32,
        /// Corner radii in device pixels, as for
        /// [`DisplayItem::FillRoundedRect`]. They must already fit the
        /// rectangle.
        radii: [[f32; 2]; 4],
        /// Standard deviation of the blur in device pixels (half the CSS
        /// blur radius); 0 draws an anti-aliased sharp edge.
        sigma: f32,
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
