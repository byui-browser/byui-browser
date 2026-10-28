//! Read-only loaded HTML snapshot, without fetching or executing page content.

use ab_glyph::FontVec;
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

const LINE_HEIGHT: f64 = 18.0;

/// Cached source layout and scrolling state; source is never executed or fetched.
pub(crate) struct PageSource {
    pub(crate) html: String,
    rows: Vec<(usize, String)>,
    columns: usize,
    offset: usize,
    visible: usize,
    wheel_remainder: f64,
    panel: Option<Rect>,
}

impl PageSource {
    /// Copies the currently loaded HTML; this view is not a live DOM inspector.
    pub(crate) fn new(html: &str) -> Self {
        Self {
            html: html.to_owned(),
            rows: Vec::new(),
            columns: 0,
            offset: 0,
            visible: 0,
            wheel_remainder: 0.0,
            panel: None,
        }
    }

    /// Rewraps only when the available column count changes; keeps the source
    /// line at the top where possible, then clamps to the new viewport.
    pub(crate) fn resize(&mut self, width: f64, height: f64) {
        let x = 16.0_f64.min(width / 4.0);
        let y = 60.0_f64.min(height / 4.0);
        self.panel = Rect::from_xywh(
            x as f32,
            y as f32,
            (width - x * 2.0) as f32,
            (height - y - 16.0_f64.min(height / 4.0)) as f32,
        );
        let Some(panel) = self.panel else {
            return;
        };
        let columns = ((f64::from(panel.width()) - 88.0) / 9.0).max(1.0) as usize;
        if columns != self.columns {
            let anchor = self.rows.get(self.offset).map_or(0, |row| row.0);
            let previous_start = self
                .rows
                .iter()
                .position(|row| row.0 == anchor)
                .unwrap_or(0);
            let character_offset = self.offset.saturating_sub(previous_start) * self.columns;
            self.columns = columns;
            self.rows = self
                .html
                .lines()
                .enumerate()
                .flat_map(|(line_number, line)| {
                    let chars: Vec<char> = line.replace('\t', "    ").chars().collect();
                    if chars.is_empty() {
                        vec![(line_number, String::new())]
                    } else {
                        chars
                            .chunks(columns)
                            .map(|chunk| (line_number, chunk.iter().collect()))
                            .collect()
                    }
                })
                .collect();
            let start = self
                .rows
                .iter()
                .position(|row| row.0 >= anchor)
                .unwrap_or(0);
            let count = self
                .rows
                .iter()
                .skip(start)
                .take_while(|row| row.0 == anchor)
                .count();
            self.offset = start + (character_offset / columns).min(count.saturating_sub(1));
        }
        self.visible = ((f64::from(panel.height()) - 126.0) / LINE_HEIGHT).max(0.0) as usize;
        self.offset = self.offset.min(self.max_offset());
    }

    fn max_offset(&self) -> usize {
        self.rows.len().saturating_sub(self.visible.max(1))
    }

    /// Moves through cached wrapped rows, clamping at both ends.
    pub(crate) fn scroll(&mut self, lines: isize, width: f64, height: f64) {
        self.resize(width, height);
        self.offset = self
            .offset
            .saturating_add_signed(lines)
            .min(self.max_offset());
        self.wheel_remainder = 0.0;
    }

    /// Scrolls by a viewport of wrapped rows, retaining one row of context.
    pub(crate) fn scroll_page(&mut self, direction: isize, width: f64, height: f64) {
        self.resize(width, height);
        self.scroll(
            direction * self.visible.saturating_sub(1).max(1) as isize,
            width,
            height,
        );
    }

    /// Accumulates fractional row movement so small trackpad events are retained.
    pub(crate) fn wheel(&mut self, rows: f64, width: f64, height: f64) {
        self.resize(width, height);
        self.wheel_remainder += rows;
        let whole = self.wheel_remainder.trunc() as isize;
        self.wheel_remainder -= whole as f64;
        self.offset = self
            .offset
            .saturating_add_signed(whole)
            .min(self.max_offset());
    }

    fn close_rect(&self) -> Option<Rect> {
        let panel = self.panel?;
        if panel.width() < 100.0 || panel.height() < 40.0 {
            return None;
        }
        Rect::from_xywh(panel.right() - 76.0, panel.top() + 8.0, 64.0, 26.0)
    }

    /// Hit-tests the visible Close button using logical pointer coordinates.
    pub(crate) fn close_at(&self, cursor: Option<(f64, f64)>) -> bool {
        let (Some(rect), Some((x, y))) = (self.close_rect(), cursor) else {
            return false;
        };
        x >= f64::from(rect.left())
            && x < f64::from(rect.right())
            && y >= f64::from(rect.top())
            && y < f64::from(rect.bottom())
    }

    /// Draws the panel within the window using the latest wrapped layout.
    pub(crate) fn draw(&mut self, frame: &mut Pixmap, scale: f64, font: Option<&FontVec>) {
        self.resize(
            f64::from(frame.width()) / scale,
            f64::from(frame.height()) / scale,
        );
        let Some(panel) = self.panel else {
            return;
        };
        let transform = Transform::from_scale(scale as f32, scale as f32);
        let mut paint = Paint::default();
        paint.set_color_rgba8(245, 247, 250, 255);
        frame.fill_rect(panel, &paint, transform, None);
        if let Some(close) = self.close_rect() {
            paint.set_color_rgba8(210, 222, 240, 255);
            frame.fill_rect(close, &paint, transform, None);
        }
        let Some(font) = font else {
            return;
        };
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
        let left = panel.left() + 12.0;
        let right = panel.right() - 28.0;
        if panel.height() < 126.0 {
            return;
        }
        let title_right = self.close_rect().map_or(right, |close| close.left() - 8.0);
        text(
            "Page source (loaded HTML)",
            left,
            panel.top() + 25.0,
            title_right,
            Color::BLACK,
        );
        if let Some(close) = self.close_rect() {
            text(
                "Close",
                close.left() + 10.0,
                close.top() + 18.0,
                close.right() - 4.0,
                Color::BLACK,
            );
        }
        text(
            "Loaded HTML, not an element inspector",
            left,
            panel.top() + 48.0,
            right,
            Color::BLACK,
        );
        text(
            "Scroll / arrows / PgUp PgDn / Home End | Esc closes",
            left,
            panel.top() + 70.0,
            right,
            Color::BLACK,
        );
        for (index, (line_number, row)) in self
            .rows
            .iter()
            .skip(self.offset)
            .take(self.visible)
            .enumerate()
        {
            let y = panel.top() + 96.0 + index as f32 * LINE_HEIGHT as f32;
            text(
                &format!("{}", line_number + 1),
                left,
                y,
                left + 38.0,
                Color::from_rgba8(100, 110, 125, 255),
            );
            text(
                row,
                left + 48.0,
                y,
                right,
                Color::from_rgba8(25, 50, 90, 255),
            );
        }
        let end = (self.offset + self.visible).min(self.rows.len());
        let start = if end == 0 { 0 } else { self.offset + 1 };
        text(
            &format!("Rows {start}-{end} of {}", self.rows.len()),
            left,
            panel.bottom() - 10.0,
            right,
            Color::BLACK,
        );
        // A position indicator alongside the source; scrolling uses wheel/keys.
        let track_height = (panel.height() - 126.0).max(0.0);
        let thumb_height = (track_height * self.visible as f32 / self.rows.len().max(1) as f32)
            .clamp(8.0_f32.min(track_height), track_height);
        let fraction = self.offset as f32 / self.max_offset().max(1) as f32;
        if let Some(thumb) = Rect::from_xywh(
            panel.right() - 12.0,
            panel.top() + 82.0 + fraction * (track_height - thumb_height),
            4.0,
            thumb_height,
        ) {
            paint.set_color_rgba8(110, 130, 160, 255);
            frame.fill_rect(thumb, &paint, transform, None);
        }
    }
}

/// Prefers a system monospace font, falling back to the shell font at call sites.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
pub(crate) fn load_font() -> Option<FontVec> {
    crate::toolbar::load_ui_font([
        "/System/Library/Fonts/Menlo.ttc",
        "C:/Windows/Fonts/consola.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_panel_renders_at_each_scale_and_restores_cleanly() {
        for scale in [1.0, 2.0] {
            let base =
                crate::toolbar::compose(800, 600, scale, &Default::default(), "", None, None)
                    .unwrap();
            let mut frame = base.clone();
            PageSource::new("<html>source</html>").draw(&mut frame, scale, None);
            assert_ne!(frame.data(), base.data());
            let restored =
                crate::toolbar::compose(800, 600, scale, &Default::default(), "", None, None)
                    .unwrap();
            assert_eq!(restored.data(), base.data());
        }
    }

    #[test]
    fn source_wraps_unicode_and_scrolls_without_changing_the_snapshot() {
        let html = "<p>é日本語</p>\n\n<script>alert('source only')</script>";
        let mut source = PageSource::new(html);
        source.resize(100.0, 180.0);
        assert!(source.rows.len() > 3);
        assert!(source.rows.iter().any(|row| row.1.is_empty()));
        assert_eq!(
            source
                .rows
                .iter()
                .map(|row| row.1.as_str())
                .collect::<String>(),
            html.replace('\n', "")
        );
        source.scroll(1000, 100.0, 180.0);
        assert_eq!(source.offset, source.max_offset());
        source.scroll(-1000, 100.0, 180.0);
        assert_eq!(source.offset, 0);
        assert_eq!(source.html, html);
    }

    #[test]
    fn fractional_trackpad_events_accumulate_and_scroll_both_directions() {
        let mut source = PageSource::new(&"<p>line</p>\n".repeat(500));
        for _ in 0..4 {
            source.wheel(0.25, 800.0, 600.0);
        }
        assert_eq!(source.offset, 1);
        for _ in 0..4 {
            source.wheel(-0.25, 800.0, 600.0);
        }
        assert_eq!(source.offset, 0);
        source.scroll(isize::MAX, 800.0, 600.0);
        assert_eq!(source.offset + source.visible, source.rows.len());
        source.scroll(isize::MIN, 800.0, 600.0);
        assert_eq!(source.offset, 0);
    }

    #[test]
    fn paging_uses_viewport_and_resize_preserves_a_long_lines_reading_position() {
        let mut source = PageSource::new(&"0123456789".repeat(1000));
        source.resize(800.0, 600.0);
        source.scroll_page(1, 800.0, 600.0);
        assert_eq!(source.offset, source.visible - 1);
        let character = source.offset * source.columns;
        source.resize(420.0, 280.0);
        assert!(source.offset * source.columns <= character);
        assert!(character - source.offset * source.columns < source.columns);
        let cached = source.rows.as_ptr();
        source.wheel(0.25, 420.0, 280.0);
        assert_eq!(
            source.rows.as_ptr(),
            cached,
            "scrolling must reuse the wrapped text"
        );
    }

    #[test]
    fn resizing_reflows_clamps_scroll_and_keeps_close_inside_window() {
        let mut source = PageSource::new(&"long HTML content with many words ".repeat(1000));
        source.scroll(isize::MAX, 800.0, 600.0);
        for (width, height) in [
            (420.0, 280.0),
            (1200.0, 900.0),
            (200.0, 160.0),
            (40.0, 40.0),
        ] {
            source.resize(width, height);
            assert!(source.offset <= source.max_offset());
            let panel = source.panel.unwrap();
            assert!(panel.right() <= width as f32 && panel.bottom() <= height as f32);
            if let Some(close) = source.close_rect() {
                assert!(close.right() <= panel.right() && close.bottom() <= panel.bottom());
                assert!(source.close_at(Some((
                    f64::from(close.left() + 1.0),
                    f64::from(close.top() + 1.0)
                ))));
            }
            assert!(!source.close_at(Some((0.0, 0.0))));
        }
    }
}
