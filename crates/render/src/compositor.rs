//! Software compositor and rasterization.

use crate::{DisplayItem, Frame, fonts::glyph};

/// A software compositor for a fixed device-pixel viewport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compositor {
    width: u32,
    height: u32,
}

impl Compositor {
    /// Opaque white used to clear a new frame.
    pub const CLEAR_COLOR: [u8; 4] = [255, 255, 255, 255];

    /// Creates a compositor with a device-pixel viewport.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Rasterizes paint commands into a new RGBA8 frame.
    pub fn compose(&self, items: &[DisplayItem]) -> Frame {
        let mut frame = Frame {
            width: self.width,
            height: self.height,
            pixels: Self::CLEAR_COLOR.repeat((self.width * self.height) as usize),
        };
        for item in items {
            match item {
                DisplayItem::FillRect {
                    x,
                    y,
                    width,
                    height,
                    rgba,
                } => {
                    let x_end = x.saturating_add(*width).min(self.width);
                    let y_end = y.saturating_add(*height).min(self.height);
                    for py in *y..y_end {
                        for px in *x..x_end {
                            self.put_pixel(&mut frame, px, py, *rgba);
                        }
                    }
                }
                DisplayItem::Text { x, y, text, rgba } => {
                    let mut cursor_x = *x;
                    for character in text.chars() {
                        if let Some(glyph) = glyph(character) {
                            for (row, bits) in glyph.iter().enumerate() {
                                for column in 0..5 {
                                    if bits & (1 << (4 - column)) != 0 {
                                        self.put_pixel(
                                            &mut frame,
                                            cursor_x + column,
                                            *y + row as u32,
                                            *rgba,
                                        );
                                    }
                                }
                            }
                        }
                        cursor_x += 6;
                    }
                }
            }
        }
        frame
    }

    fn put_pixel(&self, frame: &mut Frame, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = ((y * self.width + x) * 4) as usize;
        frame.pixels[offset..offset + 4].copy_from_slice(&rgba);
    }

    /// Rasterizes a paint crate display list into a new frame.
    pub fn compose_paint(&self, list: &paint::DisplayList) -> Frame {
        let items = list
            .items
            .iter()
            .map(|item| match item {
                paint::DisplayItem::FillRect { rect, color } => DisplayItem::FillRect {
                    x: rect.x.max(0.0) as u32,
                    y: rect.y.max(0.0) as u32,
                    width: rect.width.max(0.0) as u32,
                    height: rect.height.max(0.0) as u32,
                    rgba: [color.r, color.g, color.b, color.a],
                },
                paint::DisplayItem::Text { x, y, text, color } => DisplayItem::Text {
                    x: (*x).max(0.0) as u32,
                    y: (*y).max(0.0) as u32,
                    text: text.clone(),
                    rgba: [color.r, color.g, color.b, color.a],
                },
            })
            .collect::<Vec<_>>();
        self.compose(&items)
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
