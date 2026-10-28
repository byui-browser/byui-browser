//! Paint data types.

pub use layout::Rect;

/// An RGBA color with 8 bits per channel and straight (non-premultiplied)
/// alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel, from 0 (transparent) to 255 (opaque).
    pub a: u8,
}

impl From<css::CssColor> for Color {
    fn from(color: css::CssColor) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
            a: color.a,
        }
    }
}

/// Input to [`crate::build_display_list`]: a rectangle in CSS pixels with an
/// optional background.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintBox {
    /// Rectangle to paint.
    pub rect: Rect,
    /// Optional background color.
    pub background: Option<Color>,
}
