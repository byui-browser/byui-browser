//! Finished pixel-frame representation.

/// A finished RGBA8 frame in row-major device-pixel order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Frame width in device pixels.
    pub width: u32,
    /// Frame height in device pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8 pixel data.
    pub pixels: Vec<u8>,
}
