//! Software compositor and rasterization.

use crate::{
    BitmapFont, DisplayItem, FontCollection, Frame, ImageStore, OutlineFont, fonts::glyph,
};

/// Which fonts [`Compositor::compose_scaled`] draws text with.
#[derive(Clone, Copy)]
enum FontChoice<'a> {
    /// One font for every run.
    Single(&'a OutlineFont),
    /// The face each run's [`paint::FontId`] names.
    Collection(&'a FontCollection),
}

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
        log::debug!(
            "compositing frame {}x{} from {} items",
            self.width,
            self.height,
            items.len()
        );
        if self.width == 0 || self.height == 0 {
            log::debug!(
                "compositing into a zero-size surface ({}x{})",
                self.width,
                self.height
            );
        }
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
                    log::trace!("fill rect {width}x{height} at ({x}, {y}) rgba {rgba:?}");
                    let x_end = x.saturating_add(*width).min(self.width);
                    let y_end = y.saturating_add(*height).min(self.height);
                    for py in *y..y_end {
                        for px in *x..x_end {
                            self.put_pixel(&mut frame, px, py, *rgba);
                        }
                    }
                }
                DisplayItem::FillRoundedRect {
                    left,
                    top,
                    right,
                    bottom,
                    radii,
                    rgba,
                } => self.fill_rounded_rect(
                    &mut frame,
                    [*left, *top, *right, *bottom],
                    *radii,
                    *rgba,
                ),
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

    /// Fills the rectangle `[left, top, right, bottom]` (device pixels) with
    /// elliptical corners. Pixels on a curve are blended by the fraction of
    /// a 4x4 grid of samples that falls inside the shape.
    fn fill_rounded_rect(
        &self,
        frame: &mut Frame,
        [left, top, right, bottom]: [f32; 4],
        radii: [[f32; 2]; 4],
        rgba: [u8; 4],
    ) {
        log::trace!("fill rounded rect ({left}, {top})-({right}, {bottom}) radii {radii:?}");
        if right <= left || bottom <= top {
            return;
        }
        // Each corner as (center x, center y, radius x, radius y, which
        // side of the center the curve is on).
        let [tl, tr, br, bl] = radii;
        let corners = [
            (left + tl[0], top + tl[1], tl, -1.0, -1.0),
            (right - tr[0], top + tr[1], tr, 1.0, -1.0),
            (right - br[0], bottom - br[1], br, 1.0, 1.0),
            (left + bl[0], bottom - bl[1], bl, -1.0, 1.0),
        ];
        let inside = |x: f32, y: f32| {
            corners.iter().all(|&(cx, cy, [rx, ry], sx, sy)| {
                let (dx, dy) = ((x - cx) * sx, (y - cy) * sy);
                if rx <= 0.0 || ry <= 0.0 || dx <= 0.0 || dy <= 0.0 {
                    return true;
                }
                (dx / rx).powi(2) + (dy / ry).powi(2) <= 1.0
            })
        };
        let in_corner = |px: f32, py: f32| {
            corners.iter().any(|&(cx, cy, [rx, ry], sx, sy)| {
                rx > 0.0
                    && ry > 0.0
                    && (if sx < 0.0 { px < cx } else { px + 1.0 > cx })
                    && (if sy < 0.0 { py < cy } else { py + 1.0 > cy })
            })
        };
        let range = |start: f32, end: f32, limit: u32| {
            start.floor().clamp(0.0, limit as f32) as u32
                ..end.ceil().clamp(0.0, limit as f32) as u32
        };
        const SAMPLES: u32 = 4;
        for py in range(top, bottom, self.height) {
            for px in range(left, right, self.width) {
                let (x, y) = (px as f32, py as f32);
                let coverage = if in_corner(x, y) {
                    let mut hits = 0;
                    for sy in 0..SAMPLES {
                        for sx in 0..SAMPLES {
                            let sample_x = x + (sx as f32 + 0.5) / SAMPLES as f32;
                            let sample_y = y + (sy as f32 + 0.5) / SAMPLES as f32;
                            if (left..right).contains(&sample_x)
                                && (top..bottom).contains(&sample_y)
                                && inside(sample_x, sample_y)
                            {
                                hits += 1;
                            }
                        }
                    }
                    hits as f32 / (SAMPLES * SAMPLES) as f32
                } else {
                    let center = (x + 0.5, y + 0.5);
                    if (left..right).contains(&center.0) && (top..bottom).contains(&center.1) {
                        1.0
                    } else {
                        0.0
                    }
                };
                let alpha = (coverage * f32::from(rgba[3])).round() as u8;
                if alpha > 0 {
                    self.put_pixel(frame, px, py, [rgba[0], rgba[1], rgba[2], alpha]);
                }
            }
        }
    }

    /// Converts a paint rounded rectangle to a device-pixel
    /// [`DisplayItem::FillRoundedRect`] at `scale` device pixels per CSS
    /// pixel. Edges snap to device pixel boundaries like
    /// [`DisplayItem::FillRect`] edges; radii shrink again if snapping made
    /// the rectangle too small for them.
    fn rounded_rect_item(
        rect: &paint::Rect,
        radii: &paint::CornerRadii,
        color: &paint::Color,
        scale: f32,
    ) -> DisplayItem {
        let edge = |value: f32| (value * scale).round();
        let (left, top, right, bottom) = (
            edge(rect.x),
            edge(rect.y),
            edge(rect.right()),
            edge(rect.bottom()),
        );
        let mut radii = radii
            .scaled(scale)
            .into_array()
            .map(|radius| [radius.horizontal, radius.vertical]);
        let (width, height) = ((right - left).max(0.0), (bottom - top).max(0.0));
        let fit = |length: f32, a: f32, b: f32| {
            if a + b > length {
                length / (a + b)
            } else {
                1.0
            }
        };
        let factor = fit(width, radii[0][0], radii[1][0])
            .min(fit(width, radii[3][0], radii[2][0]))
            .min(fit(height, radii[0][1], radii[3][1]))
            .min(fit(height, radii[1][1], radii[2][1]));
        for radius in radii.iter_mut().flatten() {
            *radius *= factor;
        }
        DisplayItem::FillRoundedRect {
            left,
            top,
            right,
            bottom,
            radii,
            rgba: [color.r, color.g, color.b, color.a],
        }
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
            log::trace!("skipping text at non-positive font size {font_size}");
            return;
        }
        log::trace!(
            "draw bitmap text {} chars at ({x}, {baseline}) size {font_size}",
            text.chars().count()
        );
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
            } else {
                log::trace!("no bitmap glyph for {character:?}; leaving a blank cell");
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
        log::debug!(
            "compositing paint display list with {} items",
            list.items.len()
        );
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
                paint::DisplayItem::FillRoundedRect { rect, radii, color } => {
                    Some(Self::rounded_rect_item(rect, radii, color, 1.0))
                }
                paint::DisplayItem::Text {
                    x,
                    baseline,
                    font_size,
                    text,
                    color,
                    ..
                } => Some(DisplayItem::Text {
                    x: x.round(),
                    baseline: baseline.round(),
                    font_size: *font_size,
                    text: text.clone(),
                    rgba: [color.r, color.g, color.b, color.a],
                }),
                paint::DisplayItem::Image { image, .. } => {
                    log::trace!("no image store to draw {image:?}; use compose_page");
                    None
                }
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
    ///
    /// Every text run uses `font`, whatever its [`paint::FontId`].
    /// [`paint::DisplayItem::Image`] items are skipped; use
    /// [`Self::compose_page`] to draw them.
    pub fn compose_paint_with_font(
        &self,
        list: &paint::DisplayList,
        font: &OutlineFont,
        scale: f32,
    ) -> Frame {
        self.compose_scaled(list, FontChoice::Single(font), None, scale)
    }

    /// Rasterizes a paint crate display list into a new frame, drawing
    /// pictures from `images` and each text run with the face `fonts` has
    /// for its [`paint::FontId`].
    ///
    /// This is [`Self::compose_paint_with_font`] plus images and web fonts.
    /// Lay the document out with the same `fonts` so each run uses the face
    /// layout measured. Runs whose face is the [`BitmapFont`] fallback (no
    /// default outline font) are drawn with it, scaled to device pixels.
    ///
    /// Each [`paint::DisplayItem::Image`] is stretched to fill its rectangle,
    /// whose edges snap to device pixel boundaries like rectangle fills. The
    /// picture is resampled (bitmaps) or rasterized (SVG) at that device size
    /// and blended over what is already drawn. Images missing from `images`
    /// draw nothing.
    pub fn compose_page(
        &self,
        list: &paint::DisplayList,
        fonts: &FontCollection,
        images: &ImageStore,
        scale: f32,
    ) -> Frame {
        self.compose_scaled(list, FontChoice::Collection(fonts), Some(images), scale)
    }

    fn compose_scaled(
        &self,
        list: &paint::DisplayList,
        fonts: FontChoice<'_>,
        images: Option<&ImageStore>,
        scale: f32,
    ) -> Frame {
        log::debug!(
            "compositing frame {}x{} from {} items at scale {scale}",
            self.width,
            self.height,
            list.items.len()
        );
        if self.width == 0 || self.height == 0 {
            log::debug!(
                "compositing into a zero-size surface ({}x{})",
                self.width,
                self.height
            );
        }
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
                paint::DisplayItem::FillRoundedRect { rect, radii, color } => {
                    if let DisplayItem::FillRoundedRect {
                        left,
                        top,
                        right,
                        bottom,
                        radii,
                        rgba,
                    } = Self::rounded_rect_item(rect, radii, color, scale)
                    {
                        self.fill_rounded_rect(&mut frame, [left, top, right, bottom], radii, rgba);
                    }
                }
                paint::DisplayItem::Text {
                    x,
                    baseline,
                    font_size,
                    font,
                    text,
                    color,
                } => {
                    let rgba = [color.r, color.g, color.b, color.a];
                    let outline = match fonts {
                        FontChoice::Single(font) => Some(font),
                        FontChoice::Collection(fonts) => fonts.outline(*font),
                    };
                    match outline {
                        Some(font) => self.draw_outline_text(
                            &mut frame,
                            font,
                            *font_size * scale,
                            *x * scale,
                            (*baseline * scale).round(),
                            text,
                            rgba,
                        ),
                        None => self.draw_text(
                            &mut frame,
                            (*x * scale).round(),
                            (*baseline * scale).round(),
                            *font_size * scale,
                            text,
                            rgba,
                        ),
                    }
                }
                paint::DisplayItem::Image { rect, image } => match images {
                    Some(images) => self.draw_image(&mut frame, images, image, rect, scale),
                    None => log::trace!("no image store to draw {image:?}"),
                },
            }
        }
        frame
    }

    /// Draws `key`'s picture stretched over `rect` (CSS pixels) at `scale`
    /// device pixels per CSS pixel.
    fn draw_image(
        &self,
        frame: &mut Frame,
        images: &ImageStore,
        key: &paint::ImageKey,
        rect: &paint::Rect,
        scale: f32,
    ) {
        let edge = |value: f32| (value * scale).round();
        let (left, top) = (edge(rect.x), edge(rect.y));
        let (right, bottom) = (edge(rect.right()), edge(rect.bottom()));
        if right <= left || bottom <= top {
            return;
        }
        // Skip pictures entirely outside the frame before resampling them.
        if right <= 0.0 || bottom <= 0.0 || left >= self.width as f32 || top >= self.height as f32 {
            return;
        }
        let (width, height) = ((right - left) as u32, (bottom - top) as u32);
        log::trace!("draw image {key:?} {width}x{height} at ({left}, {top})");
        // Blends `sample(x, y)`, in picture pixels, over the visible part.
        let blit = |frame: &mut Frame, sample: &dyn Fn(u32, u32) -> [u8; 4]| {
            let columns = left.max(0.0) as u32..(right as u32).min(self.width);
            for py in top.max(0.0) as u32..(bottom as u32).min(self.height) {
                for px in columns.clone() {
                    let pixel = sample(px - left as u32, py - top as u32);
                    if pixel[3] > 0 {
                        self.put_pixel(frame, px, py, pixel);
                    }
                }
            }
        };
        if let Some(bitmap) = images.scaled(key, width, height) {
            blit(frame, &|x, y| bitmap.pixel(x, y));
        } else if let Some(bitmap) = images.original(key) {
            // Too large to resample into the cache: sample the nearest
            // source pixel directly.
            let (sx, sy) = (
                bitmap.width as f32 / width as f32,
                bitmap.height as f32 / height as f32,
            );
            blit(frame, &|x, y| {
                let source_x = ((x as f32 + 0.5) * sx) as u32;
                let source_y = ((y as f32 + 0.5) * sy) as u32;
                bitmap.pixel(
                    source_x.min(bitmap.width - 1),
                    source_y.min(bitmap.height - 1),
                )
            });
        } else {
            log::trace!("no picture for {key:?}; nothing drawn");
        }
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
            log::trace!("skipping text at non-positive font size {font_size}");
            return;
        }
        log::trace!(
            "draw outline text {} chars at ({x}, {baseline}) size {font_size}",
            text.chars().count()
        );
        let scaled = font.font().as_scaled(font.px_scale(font_size));
        let mut caret = x;
        for character in text.chars() {
            let id = scaled.glyph_id(character);
            let glyph =
                id.with_scale_and_position(scaled.scale(), ab_glyph::point(caret, baseline));
            caret += scaled.h_advance(id);
            let Some(outline) = font.font().outline_glyph(glyph) else {
                log::trace!("no outline for {character:?}; nothing drawn");
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
    fn rounded_rects_leave_corners_clear_and_fill_the_middle() {
        let radius = paint::CornerRadius {
            horizontal: 4.0,
            vertical: 4.0,
        };
        let list = paint::DisplayList {
            items: vec![paint::DisplayItem::FillRoundedRect {
                rect: paint::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                radii: paint::CornerRadii::from_array([radius; 4]),
                color: paint::Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
            }],
        };
        for scale in [1.0, 2.0] {
            let size = (10.0 * scale) as u32;
            let compositor = Compositor::new(size, size);
            // `compose_paint_with_font` draws through the same helper, but
            // needs a font file, so the scaled case calls it directly.
            let frame = if scale == 1.0 {
                compositor.compose_paint(&list)
            } else {
                let paint::DisplayItem::FillRoundedRect { rect, radii, color } = &list.items[0]
                else {
                    unreachable!()
                };
                compositor.compose(&[Compositor::rounded_rect_item(rect, radii, color, scale)])
            };
            let pixel = |x: u32, y: u32| {
                let offset = ((y * size + x) * 4) as usize;
                frame.pixels[offset..offset + 4].to_vec()
            };
            let last = size - 1;
            for (x, y) in [(0, 0), (last, 0), (0, last), (last, last)] {
                assert_eq!(pixel(x, y), Compositor::CLEAR_COLOR, "corner at {scale}x");
            }
            assert_eq!(pixel(size / 2, 0), [0, 0, 0, 255]);
            assert_eq!(pixel(0, size / 2), [0, 0, 0, 255]);
            assert_eq!(pixel(size / 2, size / 2), [0, 0, 0, 255]);
            // A pixel the curve crosses is blended rather than hard-edged.
            let edge = pixel(scale as u32, scale as u32);
            assert!(edge[0] > 0 && edge[0] < 255, "anti-aliased edge {edge:?}");
        }
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
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
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
                font: paint::FontId::DEFAULT,
                text: "Hi".into(),
                color: black,
            }],
        };
        let scale = 2.0;
        let frame = Compositor::new(120, 60).compose_paint_with_font(&list, &font, scale);
        let inked: Vec<(u32, u32)> = frame
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, pixel)| *pixel != Compositor::CLEAR_COLOR)
            .map(|(index, _)| (index as u32 % 120, index as u32 / 120))
            .collect();
        assert!(!inked.is_empty());
        // Anti-aliased edges produce partially covered (gray) pixels.
        assert!(
            frame
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .copied()
                .any(|p| p[0] > 0 && p[0] < 255)
        );
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
            .as_chunks::<4>()
            .0
            .iter()
            .copied()
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

#[cfg(test)]
mod image_tests {
    use super::*;
    use crate::images::tests::png;

    fn key(name: &str) -> paint::ImageKey {
        paint::ImageKey::Src(name.into())
    }

    fn image_item(name: &str, x: f32, y: f32, width: f32, height: f32) -> paint::DisplayItem {
        paint::DisplayItem::Image {
            rect: paint::Rect {
                x,
                y,
                width,
                height,
            },
            image: key(name),
        }
    }

    fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * frame.width + x) * 4) as usize;
        frame.pixels[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn images_are_stretched_over_their_rect_at_device_scale() {
        let mut images = ImageStore::new();
        images.insert_encoded(key("red.png"), &png(1, 1, [255, 0, 0, 255]));
        let list = paint::DisplayList {
            items: vec![image_item("red.png", 1.0, 1.0, 2.0, 1.0)],
        };
        let frame =
            Compositor::new(8, 8).compose_page(&list, &FontCollection::default(), &images, 2.0);
        let red = frame
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| **pixel == [255, 0, 0, 255])
            .count();
        assert_eq!(red, 4 * 2);
        assert_eq!(pixel(&frame, 2, 2), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, 1, 1), Compositor::CLEAR_COLOR);
    }

    #[test]
    fn translucent_images_blend_and_missing_images_draw_nothing() {
        let mut images = ImageStore::new();
        images.insert_encoded(key("veil.png"), &png(2, 2, [0, 0, 0, 128]));
        let list = paint::DisplayList {
            items: vec![
                image_item("veil.png", 0.0, 0.0, 2.0, 2.0),
                image_item("missing.png", 2.0, 0.0, 2.0, 2.0),
            ],
        };
        let frame =
            Compositor::new(4, 2).compose_page(&list, &FontCollection::default(), &images, 1.0);
        assert_eq!(pixel(&frame, 0, 0), [127, 127, 127, 255]);
        assert_eq!(pixel(&frame, 3, 1), Compositor::CLEAR_COLOR);
    }

    #[test]
    fn partly_offscreen_images_are_clipped() {
        let mut images = ImageStore::new();
        images.insert_encoded(key("a.png"), &png(4, 4, [0, 0, 255, 255]));
        let list = paint::DisplayList {
            items: vec![image_item("a.png", -2.0, -2.0, 4.0, 4.0)],
        };
        let frame =
            Compositor::new(4, 4).compose_page(&list, &FontCollection::default(), &images, 1.0);
        assert_eq!(pixel(&frame, 0, 0), [0, 0, 255, 255]);
        assert_eq!(pixel(&frame, 1, 1), [0, 0, 255, 255]);
        assert_eq!(pixel(&frame, 2, 2), Compositor::CLEAR_COLOR);
    }

    #[test]
    fn compose_page_without_a_font_matches_compose_paint_for_text_and_fills() {
        let list = paint::DisplayList {
            items: vec![
                paint::DisplayItem::FillRect {
                    rect: paint::Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 5.0,
                        height: 3.0,
                    },
                    color: paint::Color {
                        r: 9,
                        g: 9,
                        b: 9,
                        a: 255,
                    },
                },
                paint::DisplayItem::Text {
                    x: 1.2,
                    baseline: 9.4,
                    font_size: 10.0,
                    font: paint::FontId::DEFAULT,
                    text: "HI".into(),
                    color: paint::Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        a: 255,
                    },
                },
            ],
        };
        let compositor = Compositor::new(16, 12);
        assert_eq!(
            compositor.compose_page(&list, &FontCollection::default(), &ImageStore::new(), 1.0),
            compositor.compose_paint(&list)
        );
    }
}
