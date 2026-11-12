//! Paint data types.

/// An RGBA color with 8 bits per channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

/// A rectangle measured in device pixels for the legacy paint-box API.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Horizontal position.
    pub x: f32,
    /// Vertical position.
    pub y: f32,
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

/// Legacy input to [`crate::build_display_list`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintBox {
    /// Rectangle to paint.
    pub rect: Rect,
    /// Optional background color.
    pub background: Option<Color>,
}
