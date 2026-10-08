//! In-process rendering demo using the layout team's HTML -> paint -> frame API.
//! This fixed document is not connected to navigation or a renderer process yet.

use layout::{Size, StyledDom, layout_tree};
use render::{Compositor, Frame};
use tiny_skia::Pixmap;

fn render_demo(width: u32, height: u32) -> Frame {
    let page = browser::welcome_page();
    let document = html::parse_raw_html(page.html.to_owned());
    let stylesheet = css::parse_stylesheet(page.css);
    assert!(
        !stylesheet.rules.is_empty(),
        "the welcome page should have a stylesheet"
    );
    let script_result = js::eval(page.js).expect("the welcome page script should execute");
    assert_eq!(
        script_result,
        js::Value::String("Welcome to BYUI Browser!".into())
    );
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

/// Renders the demo into a device-pixel pixmap sized for the page card.
/// The current compositor rasterizes at 1x; nearest-neighbor scaling keeps its
/// bitmap font at a consistent logical size on Retina. Returns `None` for an
/// empty area.
pub(super) fn render_page(width: u32, height: u32, scale_factor: f64) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let frame = render_demo(
        (width as f64 / scale_factor).ceil() as u32,
        (height as f64 / scale_factor).ceil() as u32,
    );
    for (y, row) in pixmap
        .data_mut()
        .chunks_exact_mut(width as usize * 4)
        .enumerate()
    {
        let source_y = ((y as f64 / scale_factor) as u32).min(frame.height - 1);
        for (x, pixel) in row.chunks_exact_mut(4).enumerate() {
            let source_x = ((x as f64 / scale_factor) as u32).min(frame.width - 1);
            let offset = (source_y * frame.width + source_x) as usize * 4;
            let rgba = &frame.pixels[offset..offset + 4];
            // Composite RGBA onto the white page so the card stays opaque.
            let alpha = u32::from(rgba[3]);
            let channel =
                |value: u8| ((u32::from(value) * alpha + 255 * (255 - alpha)) / 255) as u8;
            pixel.copy_from_slice(&[channel(rgba[0]), channel(rgba[1]), channel(rgba[2]), 255]);
        }
    }
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_produces_layout_background_and_renderer_text() {
        let frame = render_demo(160, 120);
        assert_eq!((frame.width, frame.height), (160, 120));
        assert_eq!(&frame.pixels[..4], &[242, 245, 250, 255]);
        assert!(frame.pixels.chunks_exact(4).any(|p| p == [30, 43, 62, 255]));
        assert_eq!(&frame.pixels[160 * 110 * 4..160 * 110 * 4 + 4], &[255; 4]);
    }

    #[test]
    fn page_is_scaled_to_device_pixels() {
        for scale in [1.0, 1.5, 2.0] {
            let (width, height) = ((160.0 * scale) as u32, (120.0 * scale) as u32);
            let page = render_page(width, height, scale).unwrap();
            assert_eq!((page.width(), page.height()), (width, height));
            let data = page.data();
            assert_eq!(&data[..4], &[242, 245, 250, 255]);
            assert!(data.chunks_exact(4).any(|p| p == [30, 43, 62, 255]));
            let bottom = ((110.0 * scale) as u32 * width) as usize * 4;
            assert_eq!(&data[bottom..bottom + 4], &[255; 4]);
        }
    }

    #[test]
    fn tiny_and_empty_pages_are_safe() {
        assert!(render_page(0, 10, 1.0).is_none());
        for (width, height) in [(1, 1), (3, 48), (17, 70)] {
            assert!(render_page(width, height, 2.0).is_some());
        }
    }
}
