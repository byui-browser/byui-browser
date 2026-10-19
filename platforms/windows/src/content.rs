//! The first Windows page surface.
//!
//! The engine crates are still scaffolding on this branch, so the shell keeps
//! the page surface deliberately small: it uses the shared compositor's
//! cleared frame and paints the current welcome banner into it.

use render::Compositor;
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

pub(super) fn render_page(
    width: u32,
    height: u32,
    _scale_factor: f64,
    page_id: u8,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let frame = Compositor::new(width, height).compose(&[]);
    pixmap.data_mut().copy_from_slice(&frame.pixels);

    let banner_height = (height / 4).max(1).min(height);
    let mut paint = Paint::default();
    let banner = if page_id == 0 {
        Color::from_rgba8(218, 235, 248, 255)
    } else {
        Color::from_rgba8(225, 239, 220, 255)
    };
    paint.set_color(banner);
    if let Some(rect) = Rect::from_xywh(0.0, 0.0, width as f32, banner_height as f32) {
        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
    }

    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_surface_has_the_expected_size_and_welcome_banner() {
        let page = render_page(160, 120, 1.0, 0).expect("non-empty page");
        assert_eq!((page.width(), page.height()), (160, 120));
        let banner = page.pixel(2, 2).unwrap();
        assert_eq!(
            (banner.red(), banner.green(), banner.blue(), banner.alpha()),
            (218, 235, 248, 255)
        );
        let page_background = page.pixel(2, 119).unwrap();
        assert_eq!(
            (
                page_background.red(),
                page_background.green(),
                page_background.blue(),
                page_background.alpha()
            ),
            (255, 255, 255, 255)
        );
    }

    #[test]
    fn empty_surface_is_safe() {
        assert!(render_page(0, 10, 1.0, 0).is_none());
    }
}
