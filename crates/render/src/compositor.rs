//! Software compositor and rasterization.

use crate::{BitmapFont, DisplayItem, Frame, OutlineFont, fonts::glyph};

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
                DisplayItem::Text {
                    x,
                    baseline,
                    font_size,
                    text,
                    rgba,
                } => self.draw_text(&mut frame, *x, *baseline, *font_size, text, *rgba),
            }
        }
        frame
    }

    /// Draws text by sampling each scaled glyph bitmap at device-pixel
    /// centers (nearest-neighbor scaling).
    fn draw_text(
        &self,
        frame: &mut Frame,
        x: f32,
        baseline: f32,
        font_size: f32,
        text: &str,
        rgba: [u8; 4],
    ) {
        let scale = BitmapFont::scale(font_size);
        if scale <= 0.0 {
            return;
        }
        let top = baseline - BitmapFont::GLYPH_HEIGHT as f32 * scale;
        let rows = top.floor().max(0.0) as u32..baseline.ceil().max(0.0) as u32;
        let mut cell_x = x;
        for character in text.chars() {
            if let Some(glyph) = glyph(character) {
                let right = cell_x + BitmapFont::GLYPH_WIDTH as f32 * scale;
                let columns = cell_x.floor().max(0.0) as u32..right.ceil().max(0.0) as u32;
                for py in rows.clone() {
                    let row = ((py as f32 + 0.5 - top) / scale).floor();
                    if !(0.0..BitmapFont::GLYPH_HEIGHT as f32).contains(&row) {
                        continue;
                    }
                    for px in columns.clone() {
                        let column = ((px as f32 + 0.5 - cell_x) / scale).floor();
                        if !(0.0..BitmapFont::GLYPH_WIDTH as f32).contains(&column) {
                            continue;
                        }
                        let bit = BitmapFont::GLYPH_WIDTH - 1 - column as u32;
                        if glyph[row as usize] & (1 << bit) != 0 {
                            self.put_pixel(frame, px, py, rgba);
                        }
                    }
                }
            }
            cell_x += BitmapFont::ADVANCE * scale;
        }
    }

    fn put_pixel(&self, frame: &mut Frame, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let offset = ((y * self.width + x) * 4) as usize;
        let destination = &mut frame.pixels[offset..offset + 4];
        let alpha = u32::from(rgba[3]);
        if alpha == 255 {
            destination.copy_from_slice(&rgba);
            return;
        }
        // Source-over compositing with straight (non-premultiplied) alpha.
        let destination_alpha = u32::from(destination[3]) * (255 - alpha) / 255;
        let out_alpha = alpha + destination_alpha;
        if out_alpha == 0 {
            destination.copy_from_slice(&[0, 0, 0, 0]);
            return;
        }
        for channel in 0..3 {
            let blended = (u32::from(rgba[channel]) * alpha
                + u32::from(destination[channel]) * destination_alpha)
                / out_alpha;
            destination[channel] = blended as u8;
        }
        destination[3] = out_alpha as u8;
    }

    /// Rasterizes a paint crate display list into a new frame.
    ///
    /// One CSS pixel maps to one device pixel. Rectangle edges are rounded to
    /// the nearest pixel boundary, so a rectangle covers the pixels whose
    /// centers it contains, and parts outside the frame are clipped.
    pub fn compose_paint(&self, list: &paint::DisplayList) -> Frame {
        let items = list
            .items
            .iter()
            .filter_map(|item| match item {
                paint::DisplayItem::FillRect { rect, color } => {
                    let edge =
                        |value: f32, limit: u32| value.round().clamp(0.0, limit as f32) as u32;
                    let (left, right) = (edge(rect.x, self.width), edge(rect.right(), self.width));
                    let (top, bottom) =
                        (edge(rect.y, self.height), edge(rect.bottom(), self.height));
                    (right > left && bottom > top).then(|| DisplayItem::FillRect {
                        x: left,
                        y: top,
                        width: right - left,
                        height: bottom - top,
                        rgba: [color.r, color.g, color.b, color.a],
                    })
                }
                paint::DisplayItem::Text {
                    x,
                    baseline,
                    font_size,
                    text,
                    color,
                } => Some(DisplayItem::Text {
                    x: x.round(),
                    baseline: baseline.round(),
                    font_size: *font_size,
                    text: text.clone(),
                    rgba: [color.r, color.g, color.b, color.a],
                }),
            })
            .collect::<Vec<_>>();
        self.compose(&items)
    }

    /// Rasterizes a paint crate display list into a new frame, drawing text
    /// with `font` and anti-aliasing glyph edges.
    ///
    /// `scale` is device pixels per CSS pixel (for example 2.0 on a Retina
    /// display); this compositor's viewport is in device pixels. Text is
    /// rasterized at device resolution, so it stays sharp at any scale.
    /// Lay the document out with the same `font` so text runs fit the
    /// widths layout measured. Rectangle edges round to the nearest device
    /// pixel boundary, and parts outside the frame are clipped.
    pub fn compose_paint_with_font(
        &self,
        list: &paint::DisplayList,
        font: &OutlineFont,
        scale: f32,
    ) -> Frame {
        let mut frame = Frame {
            width: self.width,
            height: self.height,
            pixels: Self::CLEAR_COLOR.repeat((self.width * self.height) as usize),
        };
        for item in &list.items {
            match item {
                paint::DisplayItem::FillRect { rect, color } => {
                    let edge = |value: f32, limit: u32| {
                        (value * scale).round().clamp(0.0, limit as f32) as u32
                    };
                    let rgba = [color.r, color.g, color.b, color.a];
                    for py in edge(rect.y, self.height)..edge(rect.bottom(), self.height) {
                        for px in edge(rect.x, self.width)..edge(rect.right(), self.width) {
                            self.put_pixel(&mut frame, px, py, rgba);
                        }
                    }
                }
                paint::DisplayItem::Text {
                    x,
                    baseline,
                    font_size,
                    text,
                    color,
                } => self.draw_outline_text(
                    &mut frame,
                    font,
                    *font_size * scale,
                    *x * scale,
                    (*baseline * scale).round(),
                    text,
                    [color.r, color.g, color.b, color.a],
                ),
            }
        }
        frame
    }

    /// Draws `text` with `font` at an em size of `font_size` device pixels,
    /// starting at `x` on the `baseline` (device pixels). Glyphs advance
    /// exactly as [`layout::FontMetrics`] measures them for this font.
    #[allow(clippy::too_many_arguments)]
    fn draw_outline_text(
        &self,
        frame: &mut Frame,
        font: &OutlineFont,
        font_size: f32,
        x: f32,
        baseline: f32,
        text: &str,
        rgba: [u8; 4],
    ) {
        use ab_glyph::{Font, ScaleFont};
        if font_size <= 0.0 {
            return;
        }
        let scaled = font.font().as_scaled(font.px_scale(font_size));
        let mut caret = x;
        for character in text.chars() {
            let id = scaled.glyph_id(character);
            let glyph =
                id.with_scale_and_position(scaled.scale(), ab_glyph::point(caret, baseline));
            caret += scaled.h_advance(id);
            let Some(outline) = font.font().outline_glyph(glyph) else {
                continue;
            };
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i64 + i64::from(gx);
                let py = bounds.min.y as i64 + i64::from(gy);
                if px < 0 || py < 0 {
                    return;
                }
                let alpha = (coverage.clamp(0.0, 1.0) * f32::from(rgba[3])).round() as u8;
                if alpha > 0 {
                    self.put_pixel(
                        frame,
                        px as u32,
                        py as u32,
                        [rgba[0], rgba[1], rgba[2], alpha],
                    );
                }
            });
        }
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

    #[test]
    fn text_at_ten_pixels_draws_unscaled_glyphs_above_the_baseline() {
        let frame = Compositor::new(8, 10).compose(&[DisplayItem::Text {
            x: 0.0,
            baseline: 8.0,
            font_size: BitmapFont::UNITS_PER_EM,
            text: "I".into(),
            rgba: [0, 0, 0, 255],
        }]);
        let pixel = |x: u32, y: u32| {
            let offset = ((y * 8 + x) * 4) as usize;
            &frame.pixels[offset..offset + 4]
        };
        // The top row of `I` is solid and sits 7 rows above the baseline.
        assert_eq!(pixel(0, 1), [0, 0, 0, 255]);
        assert_eq!(pixel(4, 1), [0, 0, 0, 255]);
        assert_eq!(pixel(0, 0), Compositor::CLEAR_COLOR);
        assert_eq!(pixel(0, 8), Compositor::CLEAR_COLOR);
    }

    #[test]
    fn doubled_font_size_doubles_glyph_size() {
        let frame = Compositor::new(12, 16).compose(&[DisplayItem::Text {
            x: 0.0,
            baseline: 14.0,
            font_size: BitmapFont::UNITS_PER_EM * 2.0,
            text: "I".into(),
            rgba: [0, 0, 0, 255],
        }]);
        let dark = frame
            .pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel == [0, 0, 0, 255])
            .count();
        // `I` has 5 + 5 + 5 lit cells (two bars and a 5-row stem); at scale 2
        // each cell is 4 pixels.
        assert_eq!(dark, 15 * 4);
    }

    #[test]
    fn outline_text_is_drawn_at_device_scale_within_its_advance() {
        use layout::FontMetrics;
        let Some(font) = crate::fonts::tests::system_font() else {
            return;
        };
        let black = paint::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        };
        let list = paint::DisplayList {
            items: vec![paint::DisplayItem::Text {
                x: 4.0,
                baseline: 20.0,
                font_size: 16.0,
                text: "Hi".into(),
                color: black,
            }],
        };
        let scale = 2.0;
        let frame = Compositor::new(120, 60).compose_paint_with_font(&list, &font, scale);
        let inked: Vec<(u32, u32)> = frame
            .pixels
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, pixel)| *pixel != Compositor::CLEAR_COLOR)
            .map(|(index, _)| (index as u32 % 120, index as u32 / 120))
            .collect();
        assert!(!inked.is_empty());
        // Anti-aliased edges produce partially covered (gray) pixels.
        assert!(frame.pixels.chunks_exact(4).any(|p| p[0] > 0 && p[0] < 255));
        // Ink stays within the run's measured advance and above the
        // baseline, both scaled to device pixels.
        let right = (4.0 + font.text_width("Hi", 16.0)) * scale;
        let ascent = font.vertical_metrics(16.0).ascent * scale;
        for (x, y) in inked {
            assert!(x as f32 >= 4.0 * scale - 1.0 && (x as f32) < right + 1.0);
            assert!(y as f32 >= 20.0 * scale - ascent - 1.0 && (y as f32) < 20.0 * scale + 1.0);
        }
    }

    #[test]
    fn scaled_compose_fills_rects_in_device_pixels() {
        let Some(font) = crate::fonts::tests::system_font() else {
            return;
        };
        let list = paint::DisplayList {
            items: vec![paint::DisplayItem::FillRect {
                rect: paint::Rect {
                    x: 1.0,
                    y: 1.0,
                    width: 2.0,
                    height: 1.0,
                },
                color: paint::Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            }],
        };
        let frame = Compositor::new(8, 8).compose_paint_with_font(&list, &font, 2.0);
        let red = frame
            .pixels
            .chunks_exact(4)
            .filter(|pixel| *pixel == [255, 0, 0, 255])
            .count();
        assert_eq!(red, 4 * 2);
        assert_eq!(
            &frame.pixels[(2 * 8 + 2) * 4..(2 * 8 + 2) * 4 + 4],
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn translucent_fills_blend_over_the_frame() {
        let frame = Compositor::new(1, 1).compose(&[DisplayItem::FillRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            rgba: [0, 0, 0, 128],
        }]);
        assert_eq!(&frame.pixels[..], [127, 127, 127, 255]);
    }
}
