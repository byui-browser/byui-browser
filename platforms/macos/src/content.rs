//! In-process rendering demo using the layout team's HTML -> paint -> frame API.
//! This fixed document is not connected to navigation or a renderer process yet.

use layout::{Size, StyledDom, layout_tree};
use render::{Compositor, Frame};

fn render_demo(width: u32, height: u32) -> Frame {
    let document = html::parse_raw_html("<div>Hello, browser!</div>".to_owned());
    let tree = layout_tree(
        &StyledDom::from_html_document(&document),
        Size {
            width: width as f32,
            height: height as f32,
        },
    );
    let display_list = paint::paint_document(&tree, &document);
    Compositor::new(width, height).compose_paint(&display_list)
}

/// Renders the demo into the window's content area, leaving toolbar pixels intact.
/// `width` is in device pixels. The current compositor rasterizes at 1x; nearest
/// neighbor scaling keeps its bitmap font at a consistent logical size on Retina.
pub(super) fn draw_demo(pixels: &mut [u32], width: usize, scale_factor: f64) {
    if width == 0 {
        return;
    }
    let top = super::toolbar::height_in_pixels(scale_factor).min(pixels.len() / width);
    let content = &mut pixels[top * width..];
    let height = content.len() / width;
    if height == 0 {
        return;
    }
    let frame = render_demo(
        (width as f64 / scale_factor).ceil() as u32,
        (height as f64 / scale_factor).ceil() as u32,
    );
    for (y, row) in content.chunks_exact_mut(width).enumerate() {
        let source_y = (y as f64 / scale_factor) as usize;
        for (x, pixel) in row.iter_mut().enumerate() {
            let source_x = (x as f64 / scale_factor) as usize;
            let offset = (source_y * frame.width as usize + source_x) * 4;
            let rgba = &frame.pixels[offset..offset + 4];
            // Softbuffer uses 0x00RRGGBB; composite RGBA onto the white page.
            let alpha = u32::from(rgba[3]);
            let channel = |value: u8| (u32::from(value) * alpha + 255 * (255 - alpha)) / 255;
            *pixel = (channel(rgba[0]) << 16) | (channel(rgba[1]) << 8) | channel(rgba[2]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_produces_layout_background_and_renderer_text() {
        let frame = render_demo(160, 120);
        assert_eq!((frame.width, frame.height), (160, 120));
        assert_eq!(&frame.pixels[..4], &[210, 230, 255, 255]);
        assert!(frame.pixels.chunks_exact(4).any(|p| p == [25, 45, 70, 255]));
        assert_eq!(&frame.pixels[160 * 110 * 4..160 * 110 * 4 + 4], &[255; 4]);
    }

    #[test]
    fn content_is_scaled_below_unchanged_toolbar() {
        for scale in [1.0, 1.5, 2.0] {
            let width = (160.0 * scale) as usize;
            let top = super::super::toolbar::height_in_pixels(scale);
            let mut pixels = vec![0; width * (top + (120.0 * scale) as usize)];
            super::super::toolbar::draw(&mut pixels, width, scale);
            let toolbar = pixels[..top * width].to_vec();
            draw_demo(&mut pixels, width, scale);
            assert_eq!(&pixels[..top * width], toolbar);
            assert_eq!(pixels[top * width], 0x00d2e6ff);
            assert!(pixels[top * width..].contains(&0x00192d46));
            assert_eq!(pixels[(top + (110.0 * scale) as usize) * width], 0x00ffffff);
        }
    }

    #[test]
    fn resize_and_toolbar_only_windows_are_safe() {
        draw_demo(&mut [], 0, 1.0);
        for (width, height) in [(1, 1), (3, 48), (1, 49), (17, 70), (320, 240)] {
            let mut pixels = vec![0x123456; width * height];
            draw_demo(&mut pixels, width, 1.0);
            assert!(
                pixels[..width * height.min(48)]
                    .iter()
                    .all(|p| *p == 0x123456)
            );
            if height > 48 {
                assert_eq!(pixels[width * 48], 0x00d2e6ff);
            }
        }
    }
}
