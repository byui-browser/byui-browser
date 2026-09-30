//! GPU rendering, layer composition, and compositor.
//!
//! **Owning team**: Layout & Rendering Team
//!
//! Consumes display lists / layer trees from `paint` and produces pixels
//! (or GPU command streams). Runs in the GPU / Compositor process.

#![forbid(unsafe_code)]

/// Placeholder for the drawing commands handed over by `paint`.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    FillRect {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: [u8; 4],
    },
}

/// A finished frame: RGBA8, row-major, `width * height * 4` bytes.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Composes display items into frames for a fixed viewport.
///
/// Software-only placeholder; the real compositor targets wgpu.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compositor {
    width: u32,
    height: u32,
}

impl Compositor {
    /// Background color used to clear each frame (opaque white).
    pub const CLEAR_COLOR: [u8; 4] = [255, 255, 255, 255];

    /// Creates a compositor for the given viewport in device pixels.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Renders `items` into a new frame.
    ///
    /// Currently only handles an empty item list, which yields a cleared frame.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
    // signature however your crate's public API needs.
    pub fn compose(&self, items: &[DisplayItem]) -> Frame {
        if items.is_empty() {
            let pixel_count = (self.width * self.height) as usize;
            return Frame {
                width: self.width,
                height: self.height,
                pixels: Self::CLEAR_COLOR.repeat(pixel_count),
            };
        }
        todo!("TODO(render): rasterize {} items", items.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_yields_cleared_frame_of_viewport_size() {
        let frame = Compositor::new(4, 2).compose(&[]);
        assert_eq!((frame.width, frame.height), (4, 2));
        assert_eq!(frame.pixels.len(), 4 * 2 * 4);
        assert!(
            frame
                .pixels
                .chunks(4)
                .all(|px| px == Compositor::CLEAR_COLOR)
        );
    }
}
