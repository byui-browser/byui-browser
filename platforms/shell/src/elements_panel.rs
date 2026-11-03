//! Small read-only Elements panel derived from loaded HTML, not the live JS DOM.

use ab_glyph::FontVec;
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

/// Parsed element hierarchy and independent docked-panel scrolling state.
pub(crate) struct ElementsPanel {
    rows: Vec<(usize, String)>,
    message: Option<&'static str>,
    offset: usize,
    visible: usize,
    wheel_remainder: f64,
    bounds: Option<Rect>,
}

impl ElementsPanel {
    /// Parses only the loaded HTML snapshot. No network requests or scripts run.
    pub(crate) fn new(html: Option<&str>) -> Self {
        let mut panel = Self {
            rows: Vec::new(),
            message: None,
            offset: 0,
            visible: 0,
            wheel_remainder: 0.0,
            bounds: None,
        };
        let Some(html) = html else {
            panel.message = Some("No loaded HTML available");
            return panel;
        };
        if html.len() > 1024 * 1024 {
            panel.message = Some("HTML exceeds the basic inspector's 1 MiB limit");
            return panel;
        }
        let Ok(document) = std::panic::catch_unwind(|| html::parse_raw_html(html.to_owned()))
        else {
            panel.message = Some("Could not parse loaded HTML");
            return panel;
        };
        let mut stack = vec![(document.root, 0)];
        while let Some((id, depth)) = stack.pop() {
            let Some(node) = document.node(id) else {
                continue;
            };
            let child_depth = if let html::NodeKind::Element(element) = &node.kind {
                panel
                    .rows
                    .push((depth, format!("<{}>", element.get_name())));
                depth + 1
            } else {
                depth
            };
            if panel.rows.len() >= 20_000 {
                panel.message = Some("Element list limited to 20,000 entries");
                break;
            }
            stack.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|child| (*child, child_depth)),
            );
        }
        if panel.rows.is_empty() {
            panel.message = Some("No HTML elements found in the loaded snapshot");
        }
        panel
    }

    /// Fits a bottom dock into the content area, leaving the upper page visible.
    pub(crate) fn resize(&mut self, width: f64, height: f64) {
        let available = (height - 60.0).max(0.0);
        let dock_height = (available * 0.5).min(300.0);
        self.bounds = Rect::from_xywh(
            8.0,
            (height - 8.0 - dock_height) as f32,
            (width - 16.0).max(0.0) as f32,
            dock_height as f32,
        );
        self.visible = ((dock_height - 76.0) / 18.0).max(0.0) as usize;
        self.offset = self.offset.min(self.max_offset());
    }

    fn max_offset(&self) -> usize {
        self.rows.len().saturating_sub(self.visible.max(1))
    }

    /// Scrolls by source-tree rows (fractional wheel deltas are accumulated).
    pub(crate) fn scroll(&mut self, rows: f64) {
        self.wheel_remainder += rows;
        let whole = self.wheel_remainder.trunc() as isize;
        self.wheel_remainder -= whole as f64;
        self.offset = self
            .offset
            .saturating_add_signed(whole)
            .min(self.max_offset());
    }

    /// Handles navigation keys while the inspector has keyboard focus.
    pub(crate) fn key(&mut self, key: &winit::keyboard::NamedKey) {
        use winit::keyboard::NamedKey;
        self.wheel_remainder = 0.0;
        match key {
            NamedKey::ArrowDown => self.scroll(1.0),
            NamedKey::ArrowUp => self.scroll(-1.0),
            NamedKey::PageDown => self.scroll(self.visible.max(1) as f64),
            NamedKey::PageUp => self.scroll(-(self.visible.max(1) as f64)),
            NamedKey::Home => self.offset = 0,
            NamedKey::End => self.offset = self.max_offset(),
            _ => {}
        }
    }

    fn close_rect(&self) -> Option<Rect> {
        let bounds = self.bounds?;
        if bounds.width() < 44.0 || bounds.height() < 30.0 {
            return None;
        }
        Rect::from_xywh(bounds.right() - 36.0, bounds.top() + 6.0, 28.0, 26.0)
    }

    fn in_rect(rect: Option<Rect>, cursor: Option<(f64, f64)>) -> bool {
        let (Some(rect), Some((x, y))) = (rect, cursor) else {
            return false;
        };
        x >= f64::from(rect.left())
            && x < f64::from(rect.right())
            && y >= f64::from(rect.top())
            && y < f64::from(rect.bottom())
    }

    /// Whether the pointer belongs to the panel rather than the webpage below.
    pub(crate) fn contains(&self, cursor: Option<(f64, f64)>) -> bool {
        Self::in_rect(self.bounds, cursor)
    }

    /// Hit-tests the Close (×) control, in logical pixels.
    pub(crate) fn close_at(&self, cursor: Option<(f64, f64)>) -> bool {
        Self::in_rect(self.close_rect(), cursor)
    }

    /// Draws only the bottom panel; page pixels above its bounds are preserved.
    pub(crate) fn draw(
        &mut self,
        frame: &mut Pixmap,
        scale: f64,
        dark: bool,
        font: Option<&FontVec>,
    ) {
        self.resize(
            f64::from(frame.width()) / scale,
            f64::from(frame.height()) / scale,
        );
        let Some(bounds) = self.bounds else {
            return;
        };
        let transform = Transform::from_scale(scale as f32, scale as f32);
        let mut paint = Paint::default();
        let shade = if dark { 35 } else { 245 };
        paint.set_color_rgba8(shade, shade, shade, 255);
        frame.fill_rect(bounds, &paint, transform, None);
        if let Some(close) = self.close_rect() {
            paint.set_color_rgba8(90, 110, 140, 255);
            frame.fill_rect(close, &paint, transform, None);
        }
        let Some(font) = font else {
            return;
        };
        let foreground = if dark { Color::WHITE } else { Color::BLACK };
        let mut text = |value: &str, x: f32, y: f32, right: f32, color: Color| {
            crate::toolbar::draw_text(
                frame,
                font,
                13.0 * scale as f32,
                x * scale as f32,
                y * scale as f32,
                right * scale as f32,
                color,
                value,
            );
        };
        let left = bounds.left() + 12.0;
        if bounds.height() < 60.0 {
            return;
        }
        text(
            "Elements — Basic inspector",
            left,
            bounds.top() + 24.0,
            bounds.right() - 44.0,
            foreground,
        );
        if let Some(close) = self.close_rect() {
            text(
                "×",
                close.left() + 9.0,
                close.top() + 18.0,
                close.right() - 2.0,
                Color::WHITE,
            );
        }
        text(
            "Parsed HTML snapshot, not live DOM | Esc closes",
            left,
            bounds.top() + 46.0,
            bounds.right() - 10.0,
            foreground,
        );
        if let Some(message) = self.message {
            if bounds.height() >= 88.0 {
                text(
                    message,
                    left,
                    bounds.bottom() - 8.0,
                    bounds.right() - 10.0,
                    foreground,
                );
            }
        }
        for (index, (depth, tag)) in self
            .rows
            .iter()
            .skip(self.offset)
            .take(self.visible)
            .enumerate()
        {
            let indent = (*depth as f32 * 16.0).min((bounds.width() - 100.0).max(0.0));
            text(
                tag,
                left + indent,
                bounds.top() + 70.0 + index as f32 * 18.0,
                bounds.right() - 12.0,
                foreground,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_element_relationships_without_inventing_wrappers() {
        let panel = ElementsPanel::new(Some(
            "<html><head><title>T</title></head><body><main><p>Hello</p></main></body></html>",
        ));
        assert_eq!(
            panel.rows,
            [
                (0, "<html>"),
                (1, "<head>"),
                (2, "<title>"),
                (1, "<body>"),
                (2, "<main>"),
                (3, "<p>")
            ]
            .map(|(d, s)| (d, s.to_owned()))
        );
        let fragment = ElementsPanel::new(Some("<p>Only this element</p>"));
        assert_eq!(fragment.rows, [(0, "<p>".to_owned())]);
        assert!(ElementsPanel::new(None).message.is_some());
        assert!(ElementsPanel::new(Some("")).rows.is_empty());
    }

    #[test]
    fn scrolling_and_resizing_keep_the_panel_and_close_button_visible() {
        let mut panel = ElementsPanel::new(Some(&"<p>row</p>".repeat(100)));
        for (width, height) in [(800.0, 600.0), (420.0, 280.0), (1200.0, 900.0)] {
            panel.resize(width, height);
            let bounds = panel.bounds.unwrap();
            assert!(bounds.top() > 60.0 && bounds.bottom() <= height as f32);
            assert!(bounds.right() <= width as f32);
            let close = panel.close_rect().unwrap();
            assert!(panel.close_at(Some((
                f64::from(close.left() + 1.0),
                f64::from(close.top() + 1.0)
            ))));
            panel.key(&winit::keyboard::NamedKey::End);
            assert_eq!(panel.offset, panel.max_offset());
            panel.key(&winit::keyboard::NamedKey::Home);
            for _ in 0..4 {
                panel.scroll(0.25);
            }
            assert_eq!(panel.offset, 1);
        }
    }

    #[test]
    fn drawing_leaves_upper_webpage_pixels_untouched_at_each_scale() {
        for scale in [1.0, 2.0] {
            let mut frame = Pixmap::new((800.0 * scale) as u32, (600.0 * scale) as u32).unwrap();
            frame.fill(Color::from_rgba8(180, 20, 40, 255));
            let before = frame.clone();
            let mut panel = ElementsPanel::new(Some("<body><p>Test</p></body>"));
            panel.draw(&mut frame, scale, false, None);
            let top = (f64::from(panel.bounds.unwrap().top()) * scale) as usize;
            let preserved = top * frame.width() as usize * 4;
            assert_eq!(&frame.data()[..preserved], &before.data()[..preserved]);
            assert_ne!(frame.data(), before.data());
        }
    }
}
