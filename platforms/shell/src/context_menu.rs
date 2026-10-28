//! Shared page popup: disabled Back/Forward, Reload, and page-source Inspect.

use ab_glyph::FontVec;
use tiny_skia::{Color, Paint, Pixmap, Rect, Transform};

/// Actions offered by the page popup; Inspect only shows loaded source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    Reload,
    PageSource,
}

// Disabled rows deliberately have no executable action. Navigation wiring is
// outside this feature even when BrowserState has previous/next entries.
const MENU_ITEMS: [(&str, Option<MenuAction>); 4] = [
    ("Back", None),
    ("Forward", None),
    ("Reload", Some(MenuAction::Reload)),
    ("Inspect (page source)", Some(MenuAction::PageSource)),
];

/// Popup bounds in logical pixels, independent of display scaling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContextMenu {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl ContextMenu {
    /// Anchors a four-item popup at the pointer, clamped to the window.
    pub(crate) fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        let menu_width = 220.0_f64.min(width);
        let menu_height = 136.0_f64.min(height);
        Self {
            x: x.clamp(0.0, width - menu_width),
            y: y.clamp(0.0, height - menu_height),
            width: menu_width,
            height: menu_height,
        }
    }

    /// Whether the pointer is inside the popup, including disabled rows/padding.
    pub(crate) fn contains(self, cursor: Option<(f64, f64)>) -> bool {
        cursor.is_some_and(|(x, y)| {
            x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
        })
    }

    /// Returns enabled actions only; Back/Forward cannot be selected or hovered.
    pub(crate) fn action_at(self, cursor: Option<(f64, f64)>) -> Option<MenuAction> {
        let (x, y) = cursor?;
        if x < self.x + 4.0
            || x >= self.x + self.width - 4.0
            || y < self.y + 4.0
            || y >= self.y + self.height - 4.0
        {
            return None;
        }
        let row_height = (self.height - 8.0) / MENU_ITEMS.len() as f64;
        let index = ((y - self.y - 4.0) / row_height) as usize;
        MENU_ITEMS.get(index).and_then(|(_, action)| *action)
    }

    /// Composites the menu above a freshly restored window frame.
    pub(crate) fn draw(
        self,
        frame: &mut Pixmap,
        scale: f64,
        dark: bool,
        hovered: Option<MenuAction>,
        font: Option<&FontVec>,
    ) {
        let scale = scale as f32;
        let rect = Rect::from_xywh(
            self.x as f32 * scale,
            self.y as f32 * scale,
            self.width as f32 * scale,
            self.height as f32 * scale,
        )
        .unwrap();
        let mut paint = Paint::default();
        paint.set_color_rgba8(120, 120, 130, 255);
        frame.fill_rect(rect, &paint, Transform::identity(), None);
        if let Some(inner) = Rect::from_xywh(
            rect.x() + scale,
            rect.y() + scale,
            rect.width() - 2.0 * scale,
            rect.height() - 2.0 * scale,
        ) {
            let shade = if dark { 45 } else { 250 };
            paint.set_color_rgba8(shade, shade, shade, 255);
            frame.fill_rect(inner, &paint, Transform::identity(), None);
        }
        let row_height = (rect.height() - 8.0 * scale) / MENU_ITEMS.len() as f32;
        for (index, (label, action)) in MENU_ITEMS.into_iter().enumerate() {
            let top = rect.y() + 4.0 * scale + index as f32 * row_height;
            let selected = action.is_some() && hovered == action;
            if selected
                && let Some(row) = Rect::from_xywh(
                    rect.x() + 4.0 * scale,
                    top,
                    rect.width() - 8.0 * scale,
                    row_height,
                )
            {
                paint.set_color_rgba8(40, 100, 210, 255);
                frame.fill_rect(row, &paint, Transform::identity(), None);
            }
            if let Some(font) = font {
                let color = if action.is_none() {
                    if dark {
                        Color::from_rgba8(145, 145, 155, 255)
                    } else {
                        Color::from_rgba8(140, 140, 150, 255)
                    }
                } else if dark || selected {
                    Color::WHITE
                } else {
                    Color::BLACK
                };
                crate::toolbar::draw_text(
                    frame,
                    font,
                    13.0 * scale,
                    rect.x() + 14.0 * scale,
                    top + 21.0 * scale,
                    rect.right() - 8.0 * scale,
                    color,
                    label,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_rows_have_the_requested_order_and_disabled_rows_have_no_actions() {
        assert_eq!(
            MENU_ITEMS.map(|(label, _)| label),
            ["Back", "Forward", "Reload", "Inspect (page source)"]
        );
        let menu = ContextMenu::new(100.0, 100.0, 800.0, 600.0);
        for (row, (_, expected)) in MENU_ITEMS.into_iter().enumerate() {
            let cursor = Some((120.0, 120.0 + row as f64 * 32.0));
            assert!(menu.contains(cursor));
            assert_eq!(menu.action_at(cursor), expected);
        }
        assert!(!menu.contains(Some((20.0, 20.0))));
    }

    #[test]
    fn popup_stays_inside_window_and_hit_testing_matches_position() {
        for (width, height) in [(800.0, 600.0), (100.0, 20.0)] {
            for (x, y) in [(0.0, 0.0), (width - 1.0, height - 1.0)] {
                let menu = ContextMenu::new(x, y, width, height);
                assert!(menu.x >= 0.0 && menu.y >= 0.0);
                assert!(menu.x + menu.width <= width);
                assert!(menu.y + menu.height <= height);
                assert_eq!(
                    menu.action_at(Some((
                        menu.x + 5.0,
                        menu.y + 4.0 + (menu.height - 8.0) * 0.625
                    ))),
                    Some(MenuAction::Reload)
                );
                assert_eq!(
                    menu.action_at(Some((
                        menu.x + 5.0,
                        menu.y + 4.0 + (menu.height - 8.0) * 0.875
                    ))),
                    Some(MenuAction::PageSource)
                );
                assert_eq!(menu.action_at(Some((menu.x, menu.y))), None);
                assert_eq!(menu.action_at(None), None);
            }
        }
    }

    #[test]
    fn overlay_can_be_cleared_by_restoring_the_cached_page() {
        for scale in [1.0, 2.0] {
            let base =
                crate::toolbar::compose(400, 300, scale, &Default::default(), "", None, None).unwrap();
            let mut frame = base.clone();
            ContextMenu::new(100.0, 80.0, 400.0 / scale, 300.0 / scale).draw(
                &mut frame,
                scale,
                false,
                Some(MenuAction::Reload),
                None,
            );
            assert_ne!(frame.data(), base.data());
            let restored =
                crate::toolbar::compose(400, 300, scale, &Default::default(), "", None, None).unwrap();
            assert_eq!(restored.data(), base.data());
        }
    }
}
