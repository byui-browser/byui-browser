//! Shared page popup: disabled navigation, Reload, View Page Source, Inspect,
//! and Audit Compatibility.

use ab_glyph::FontVec;
use tiny_skia::{Color, PathBuilder, Pixmap, Rect, Transform};

use crate::toolbar;

/// Independent actions offered by the page popup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    Reload,
    PageSource,
    Inspect,
    /// Opens the DevTools dock on its Compatibility tab.
    Compatibility,
}

// Disabled rows deliberately have no executable action. Navigation wiring is
// outside this feature even when BrowserState has previous/next entries.
const MENU_ITEMS: [(&str, Option<MenuAction>); 6] = [
    ("Back", None),
    ("Forward", None),
    ("Reload", Some(MenuAction::Reload)),
    ("View Page Source", Some(MenuAction::PageSource)),
    ("Inspect", Some(MenuAction::Inspect)),
    ("Audit Compatibility", Some(MenuAction::Compatibility)),
];

/// Index of the first item after the separator that splits navigation
/// (Back, Forward, Reload) from developer tools (Source, Inspect, Audit).
const SECOND_GROUP: usize = 3;
/// Menu width in logical pixels.
const WIDTH: f64 = 208.0;
/// Inner padding around the item list in logical pixels.
const PAD: f64 = 5.0;
/// Item row height in logical pixels.
const ROW: f64 = 24.0;
/// Height of the separator gap between groups in logical pixels.
const SEPARATOR: f64 = 9.0;
/// Corner radius of the menu surface in logical pixels.
const RADIUS: f32 = 8.0;
/// Full menu height in logical pixels.
const HEIGHT: f64 = 2.0 * PAD + MENU_ITEMS.len() as f64 * ROW + SEPARATOR;

/// Popup bounds in logical pixels, independent of display scaling.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContextMenu {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl ContextMenu {
    /// Anchors the popup at the pointer, clamped to the window.
    pub(crate) fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        let menu_width = WIDTH.min(width);
        let menu_height = HEIGHT.min(height);
        Self {
            x: x.clamp(0.0, width - menu_width),
            y: y.clamp(0.0, height - menu_height),
            width: menu_width,
            height: menu_height,
        }
    }

    /// Top of item `index` relative to the menu's top edge, in logical pixels.
    fn row_top(index: usize) -> f64 {
        let gap = if index >= SECOND_GROUP {
            SEPARATOR
        } else {
            0.0
        };
        PAD + index as f64 * ROW + gap
    }

    /// Whether the pointer is inside the popup, including disabled rows/padding.
    pub(crate) fn contains(self, cursor: Option<(f64, f64)>) -> bool {
        cursor.is_some_and(|(x, y)| {
            x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
        })
    }

    /// Returns enabled actions only; Back/Forward, padding, and the separator
    /// cannot be selected or hovered.
    pub(crate) fn action_at(self, cursor: Option<(f64, f64)>) -> Option<MenuAction> {
        let (x, y) = cursor?;
        if !self.contains(cursor) || x < self.x + PAD || x >= self.x + self.width - PAD {
            return None;
        }
        let y = y - self.y;
        MENU_ITEMS
            .iter()
            .enumerate()
            .find(|(index, _)| {
                let top = Self::row_top(*index);
                y >= top && y < top + ROW
            })
            .and_then(|(_, (_, action))| *action)
    }

    /// Center of item `index` in logical window coordinates, for tests.
    #[cfg(test)]
    pub(crate) fn item_center(self, index: usize) -> (f64, f64) {
        (
            self.x + self.width / 2.0,
            self.y + Self::row_top(index) + ROW / 2.0,
        )
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
        let palette = toolbar::palette(dark);
        let rgba = toolbar::rgba;
        let s = scale as f32;
        let transform = Transform::from_scale(s, s);
        let (x, y, w, h) = (
            self.x as f32,
            self.y as f32,
            self.width as f32,
            self.height as f32,
        );

        // tiny-skia has no blur: stack translucent rounded rects that grow
        // outward and drop slightly, approximating a soft drop shadow.
        let shadow_alpha = if dark { 12 } else { 4 };
        for spread in (1..=6).rev() {
            let spread = spread as f32;
            if let Some(path) = toolbar::rounded_rect(
                x - spread,
                y - spread + 3.0,
                w + 2.0 * spread,
                h + 2.0 * spread,
                RADIUS + spread,
            ) {
                toolbar::fill(frame, &path, rgba(0, 0, 0, shadow_alpha), transform);
            }
        }
        let (surface, border, separator) = if dark {
            (
                rgba(0x2a, 0x2b, 0x31, 255),
                rgba(255, 255, 255, 30),
                rgba(255, 255, 255, 22),
            )
        } else {
            (
                rgba(255, 255, 255, 255),
                rgba(15, 23, 42, 34),
                rgba(15, 23, 42, 22),
            )
        };
        if let Some(path) = toolbar::rounded_rect(x, y, w, h, RADIUS) {
            toolbar::fill(frame, &path, surface, transform);
        }
        if let Some(path) = toolbar::rounded_rect(x + 0.5, y + 0.5, w - 1.0, h - 1.0, RADIUS - 0.5)
        {
            toolbar::stroke(frame, &path, border, 1.0, transform);
        }
        let separator_y = y + (Self::row_top(SECOND_GROUP) - SEPARATOR / 2.0) as f32;
        if separator_y + 1.0 < y + h
            && let Some(rect) = Rect::from_xywh(
                x + PAD as f32 + 8.0,
                separator_y.floor(),
                w - 2.0 * PAD as f32 - 16.0,
                1.0,
            )
        {
            toolbar::fill(frame, &PathBuilder::from_rect(rect), separator, transform);
        }

        for (index, (label, action)) in MENU_ITEMS.into_iter().enumerate() {
            let top = y + Self::row_top(index) as f32;
            if top + ROW as f32 > y + h {
                break;
            }
            let selected = action.is_some() && hovered == action;
            if selected
                && let Some(path) = toolbar::rounded_rect(
                    x + PAD as f32,
                    top,
                    w - 2.0 * PAD as f32,
                    ROW as f32,
                    5.0,
                )
            {
                toolbar::fill(frame, &path, palette.accent, transform);
            }
            let Some(font) = font else {
                continue;
            };
            let color = if selected {
                Color::WHITE
            } else if action.is_none() {
                palette.icon_disabled
            } else {
                palette.text
            };
            toolbar::draw_text(
                frame,
                font,
                13.0 * s,
                (x + PAD as f32 + 10.0) * s,
                (top + ROW as f32 / 2.0 + 4.5) * s,
                (x + w - PAD as f32 - 8.0) * s,
                color,
                label,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_have_the_requested_order_and_disabled_rows_have_no_actions() {
        assert_eq!(
            MENU_ITEMS.map(|(label, _)| label),
            [
                "Back",
                "Forward",
                "Reload",
                "View Page Source",
                "Inspect",
                "Audit Compatibility"
            ]
        );
        let menu = ContextMenu::new(100.0, 100.0, 800.0, 600.0);
        for (row, (_, expected)) in MENU_ITEMS.into_iter().enumerate() {
            let cursor = Some(menu.item_center(row));
            assert!(menu.contains(cursor));
            assert_eq!(menu.action_at(cursor), expected);
        }
        assert!(!menu.contains(Some((20.0, 20.0))));
    }

    #[test]
    fn separator_and_padding_are_inert() {
        let menu = ContextMenu::new(100.0, 100.0, 800.0, 600.0);
        let (x, _) = menu.item_center(0);
        let separator = menu.y + ContextMenu::row_top(SECOND_GROUP) - SEPARATOR / 2.0;
        assert!(menu.contains(Some((x, separator))));
        assert_eq!(menu.action_at(Some((x, separator))), None);
        assert_eq!(
            menu.action_at(Some((menu.x + 1.0, menu.item_center(2).1))),
            None
        );
        assert_eq!(menu.action_at(Some((x, menu.y + 1.0))), None);
    }

    #[test]
    fn popup_stays_inside_window_and_hit_testing_matches_position() {
        for (width, height) in [(800.0, 600.0), (240.0, 160.0), (100.0, 20.0)] {
            for (x, y) in [(0.0, 0.0), (width - 1.0, height - 1.0)] {
                let menu = ContextMenu::new(x, y, width, height);
                assert!(menu.x >= 0.0 && menu.y >= 0.0);
                assert!(menu.x + menu.width <= width);
                assert!(menu.y + menu.height <= height);
                assert_eq!(menu.action_at(Some((menu.x, menu.y))), None);
                assert_eq!(menu.action_at(None), None);
                if height < HEIGHT {
                    continue;
                }
                assert_eq!(
                    menu.action_at(Some(menu.item_center(2))),
                    Some(MenuAction::Reload)
                );
                assert_eq!(
                    menu.action_at(Some(menu.item_center(3))),
                    Some(MenuAction::PageSource)
                );
            }
        }
    }

    #[test]
    fn overlay_can_be_cleared_by_restoring_the_cached_page() {
        for scale in [1.0, 2.0] {
            let base =
                crate::toolbar::compose(400, 300, scale, &Default::default(), "", None, None)
                    .unwrap();
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
                crate::toolbar::compose(400, 300, scale, &Default::default(), "", None, None)
                    .unwrap();
            assert_eq!(restored.data(), base.data());
        }
    }
}
