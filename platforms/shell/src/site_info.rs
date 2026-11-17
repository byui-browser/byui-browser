//! Read-only information about the document in the active tab.

use ab_glyph::FontVec;
use net::Url;
use tiny_skia::{PathBuilder, Pixmap, Rect, Transform};

use crate::toolbar;

/// Labels derived from the final document URL. URL schemes do not describe
/// certificate validity, page security, or the state of a live connection.
pub(crate) fn labels(url: &str) -> (String, &'static str) {
    let Ok(url) = Url::parse(url) else {
        return ("No website information".into(), "");
    };
    let detail = match url.scheme() {
        "http" => "URL scheme: HTTP",
        "https" => "URL scheme: HTTPS",
        "byui" => "Internal browser page",
        _ => "",
    };
    let host = url.host_str().unwrap_or("No website hostname");
    (host.into(), detail)
}

/// Popup geometry in logical window pixels, recomputed for each window size.
#[derive(Clone, Copy)]
pub(crate) struct Popup {
    bounds: Rect,
    close: Rect,
}

impl Popup {
    pub(crate) fn new(width: f64, height: f64, inset: f32) -> Option<Self> {
        let anchor = toolbar::site_info_anchor(width, inset)?;
        let top = toolbar::PAGE_TOP as f32;
        let width = width as f32;
        let w = width.min(272.0);
        let h = 94.0;
        if (height as f32) < top + h {
            return None;
        }
        let x = anchor.clamp(0.0, width - w);
        Some(Self {
            bounds: Rect::from_xywh(x, top, w, h)?,
            close: Rect::from_xywh(x + w - 34.0, top + 6.0, 26.0, 26.0)?,
        })
    }

    pub(crate) fn contains(self, cursor: Option<(f64, f64)>) -> bool {
        contains(self.bounds, cursor)
    }

    pub(crate) fn close_at(self, cursor: Option<(f64, f64)>) -> bool {
        contains(self.close, cursor)
    }

    #[cfg(test)]
    pub(crate) fn close_center(self) -> (f64, f64) {
        (
            f64::from(self.close.x() + 13.0),
            f64::from(self.close.y() + 13.0),
        )
    }

    pub(crate) fn draw(
        self,
        frame: &mut Pixmap,
        scale: f64,
        dark: bool,
        cursor: Option<(f64, f64)>,
        font: Option<&FontVec>,
        url: &str,
    ) {
        let palette = toolbar::palette(dark);
        let scale = scale as f32;
        let transform = Transform::from_scale(scale, scale);
        let rect = self.bounds;
        if let Some(surface) =
            toolbar::rounded_rect(rect.x(), rect.y(), rect.width(), rect.height(), 9.0)
        {
            toolbar::fill(frame, &surface, palette.panel, transform);
            toolbar::stroke(frame, &surface, palette.panel_border, 1.0, transform);
        }
        let close = self.close;
        if self.close_at(cursor)
            && let Some(path) =
                toolbar::rounded_rect(close.x(), close.y(), close.width(), close.height(), 13.0)
        {
            toolbar::fill(frame, &path, palette.hover, transform);
        }
        let (cx, cy) = (close.x() + 13.0, close.y() + 13.0);
        let mut cross = PathBuilder::new();
        for direction in [-1.0, 1.0] {
            cross.move_to(cx - 4.0, cy - 4.0 * direction);
            cross.line_to(cx + 4.0, cy + 4.0 * direction);
        }
        if let Some(path) = cross.finish() {
            toolbar::stroke(frame, &path, palette.icon, 1.6, transform);
        }
        if let Some(font) = font {
            let (host, detail) = labels(url);
            for (text, baseline, right, color) in [
                ("Site information", 24.0, close.left() - 4.0, palette.text),
                (&host, 49.0, rect.right() - 12.0, palette.text),
                (detail, 72.0, rect.right() - 12.0, palette.text_muted),
            ] {
                toolbar::draw_text(
                    frame,
                    font,
                    13.0 * scale,
                    (rect.x() + 12.0) * scale,
                    (rect.y() + baseline) * scale,
                    right * scale,
                    color,
                    text,
                );
            }
        }
    }
}

fn contains(rect: Rect, cursor: Option<(f64, f64)>) -> bool {
    cursor.is_some_and(|(x, y)| {
        x >= f64::from(rect.left())
            && x < f64::from(rect.right())
            && y >= f64::from(rect.top())
            && y < f64::from(rect.bottom())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_report_the_document_host_and_scheme_only() {
        assert_eq!(
            labels("https://user:secret@example.test:9443/page"),
            ("example.test".into(), "URL scheme: HTTPS")
        );
        assert_eq!(
            labels("http://[::1]:8080/page"),
            ("[::1]".into(), "URL scheme: HTTP")
        );
        assert_eq!(
            labels("byui://welcome/page"),
            ("welcome".into(), "Internal browser page")
        );
        assert_eq!(labels(""), ("No website information".into(), ""));
    }

    #[test]
    fn popup_paints_in_both_palettes_without_covering_the_toolbar() {
        let popup = Popup::new(420.0, 280.0, 72.0).unwrap();
        let mut surfaces = Vec::new();
        for dark in [false, true] {
            let mut frame = Pixmap::new(840, 560).unwrap();
            popup.draw(&mut frame, 2.0, dark, None, None, "https://example.test/");
            let y = (popup.bounds.top() * 2.0) as u32;
            let x = (popup.bounds.left() * 2.0) as u32 + 16;
            assert_eq!(frame.pixel(x, y - 2).unwrap().alpha(), 0);
            let surface = frame.pixel(x, y + 30).unwrap();
            assert_eq!(surface.alpha(), 255);
            surfaces.push(surface);
        }
        assert_ne!(surfaces[0], surfaces[1]);
    }

    #[test]
    fn popup_and_close_button_fit_at_narrow_and_wide_sizes() {
        for width in [330.0, 420.0, 1120.0] {
            let popup = Popup::new(width, 280.0, 72.0).unwrap();
            assert!(popup.bounds.left() >= 0.0);
            assert!(f64::from(popup.bounds.right()) <= width);
            assert!(f64::from(popup.bounds.top()) >= toolbar::PAGE_TOP);
            assert!(popup.bounds.bottom() <= 280.0);
            let close = Some((
                f64::from(popup.close.x() + 2.0),
                f64::from(popup.close.y() + 2.0),
            ));
            assert!(popup.contains(close));
            assert!(popup.close_at(close));
            assert!(!popup.contains(Some((0.0, 0.0))));
        }
        assert!(Popup::new(180.0, 280.0, 72.0).is_none());
        assert!(Popup::new(420.0, 100.0, 72.0).is_none());
    }
}
