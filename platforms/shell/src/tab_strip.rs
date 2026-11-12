//! Shared software-painted tab strip, with bounded paging for many tabs.

use crate::toolbar;
use ab_glyph::FontVec;
use chrome::TabId;
use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Transform};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) const HEIGHT: f64 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Select(TabId),
    Close(TabId),
    New,
    Previous,
    Next,
}

pub(crate) struct Strip {
    pub regions: Vec<(Action, Rect)>,
    pub start: usize,
    pub visible: usize,
    pub hovered: Option<Action>,
}

impl Strip {
    pub fn new(width: f32, ids: &[TabId], offset: usize, inset: f32) -> Self {
        let inset = inset.max(0.0);
        let available = (width - inset - 52.0).max(1.0);
        let overflow = ids.len() as f32 * 96.0 > available;
        let left = inset + if overflow { 76.0 } else { 8.0 };
        let capacity = (((width - left - 44.0) / 96.0).floor() as usize).max(1);
        let start = offset.min(ids.len().saturating_sub(capacity));
        let visible = capacity.min(ids.len().saturating_sub(start));
        let tab_width = ((width - left - 44.0) / (visible.max(1) as f32)).clamp(1.0, 200.0);
        let y = 0.0;
        let mut regions = Vec::new();
        if overflow {
            regions.push((
                Action::Previous,
                Rect::from_xywh(inset + 8.0, y + 2.0, 28.0, 28.0).unwrap(),
            ));
            regions.push((
                Action::Next,
                Rect::from_xywh(inset + 40.0, y + 2.0, 28.0, 28.0).unwrap(),
            ));
        }
        for (index, id) in ids.iter().skip(start).take(visible).enumerate() {
            let x = left + index as f32 * tab_width;
            regions.push((
                Action::Select(*id),
                Rect::from_xywh(x, y + 2.0, tab_width - 2.0, 28.0).unwrap(),
            ));
            regions.push((
                Action::Close(*id),
                Rect::from_xywh(x + tab_width - 25.0, y + 4.0, 22.0, 24.0).unwrap(),
            ));
        }
        regions.push((
            Action::New,
            Rect::from_xywh(left + visible as f32 * tab_width + 5.0, y + 2.0, 28.0, 28.0).unwrap(),
        ));
        Self {
            regions,
            start,
            visible,
            hovered: None,
        }
    }
    pub fn action_at(&self, cursor: Option<(f64, f64)>) -> Option<Action> {
        let (x, y) = cursor?;
        // Close controls take precedence over their parent tab.
        self.regions
            .iter()
            .rev()
            .find(|(_, r)| {
                x >= r.left() as f64
                    && x < r.right() as f64
                    && y >= r.top() as f64
                    && y < r.bottom() as f64
            })
            .map(|(a, _)| *a)
    }
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        scale: f64,
        dark: bool,
        active: TabId,
        entries: &[(TabId, String)],
        font: Option<&FontVec>,
    ) {
        let scale = scale as f32;
        let palette = toolbar::palette(dark);
        let mut paint = Paint::default();
        paint.set_color(palette.frame_top);
        let rect = Rect::from_xywh(0.0, 0.0, pixmap.width() as f32 / scale, HEIGHT as f32).unwrap();
        let transform = Transform::from_scale(scale, scale);
        pixmap.fill_path(
            &PathBuilder::from_rect(rect),
            &paint,
            tiny_skia::FillRule::Winding,
            transform,
            None,
        );
        for (action, rect) in &self.regions {
            if let Action::Select(id) = action {
                paint.set_color(if *id == active {
                    palette.field_hover
                } else {
                    palette.hover
                });
                pixmap.fill_path(
                    // Inset only the painted leading edge: a four-pixel visual gap
                    // without moving titles, close targets, or tab hit regions.
                    &toolbar::rounded_rect(
                        rect.x() + 2.0,
                        rect.y(),
                        rect.width() - 2.0,
                        rect.height(),
                        8.0,
                    )
                    .unwrap(),
                    &paint,
                    tiny_skia::FillRule::Winding,
                    transform,
                    None,
                );
                if *id == active {
                    paint.set_color(palette.accent);
                    if let Some(line) = Rect::from_xywh(
                        rect.x() + 2.0,
                        rect.bottom() - 2.0,
                        rect.width() - 2.0,
                        2.0,
                    ) {
                        pixmap.fill_path(
                            &PathBuilder::from_rect(line),
                            &paint,
                            tiny_skia::FillRule::Winding,
                            transform,
                            None,
                        );
                    }
                }
            }
            if matches!(action, Action::Close(_) | Action::New) {
                if *action == Action::New && self.hovered == Some(*action) {
                    let mut circle = PathBuilder::new();
                    circle.push_circle(
                        rect.x() + rect.width() / 2.0,
                        rect.y() + rect.height() / 2.0,
                        12.0,
                    );
                    paint.set_color(palette.pressed);
                    pixmap.fill_path(
                        &circle.finish().unwrap(),
                        &paint,
                        tiny_skia::FillRule::Winding,
                        transform,
                        None,
                    );
                }
                let mut path = PathBuilder::new();
                if *action == Action::New {
                    let cx = rect.x() + rect.width() / 2.0;
                    let cy = rect.y() + rect.height() / 2.0;
                    path.move_to(cx - 7.0, cy);
                    path.line_to(cx + 7.0, cy);
                    path.move_to(cx, cy - 7.0);
                    path.line_to(cx, cy + 7.0);
                } else {
                    let icon = close_icon_rect(*rect);
                    path.move_to(icon.left(), icon.top());
                    path.line_to(icon.right(), icon.bottom());
                    path.move_to(icon.right(), icon.top());
                    path.line_to(icon.left(), icon.bottom());
                }
                paint.set_color(if *action == Action::New {
                    palette.text
                } else {
                    palette.text_muted
                });
                pixmap.stroke_path(
                    &path.finish().unwrap(),
                    &paint,
                    &tiny_skia::Stroke {
                        width: 1.4,
                        line_cap: tiny_skia::LineCap::Round,
                        ..Default::default()
                    },
                    transform,
                    None,
                );
                continue;
            }
            if let Some(font) = font {
                let text = match action {
                    Action::Select(id) => entries
                        .iter()
                        .find(|(key, _)| key == id)
                        .map_or("New Tab", |(_, title)| title.as_str()),
                    Action::Close(_) => "×",
                    Action::New => "+",
                    Action::Previous => "‹",
                    Action::Next => "›",
                };
                let clip = if matches!(action, Action::Select(_)) {
                    rect.right() - 26.0
                } else {
                    rect.right() - 2.0
                };
                let fitted = if matches!(action, Action::Select(_)) {
                    fit_title(
                        text,
                        font,
                        12.0 * scale,
                        ((clip - rect.x() - 10.0) * scale).max(0.0),
                    )
                } else {
                    text.to_owned()
                };
                toolbar::draw_text(
                    pixmap,
                    font,
                    12.0 * scale,
                    (rect.x()
                        + if matches!(action, Action::Select(_)) {
                            10.0
                        } else {
                            5.0
                        })
                        * scale,
                    (rect.y() + 19.0) * scale,
                    clip * scale,
                    palette.text,
                    &fitted,
                );
            }
        }
    }
}

/// Font-independent close mark centered in the existing click target.
fn close_icon_rect(target: Rect) -> Rect {
    Rect::from_xywh(
        target.x() + target.width() / 2.0 - 4.0,
        target.y() + target.height() / 2.0 - 4.0,
        8.0,
        8.0,
    )
    .unwrap()
}

/// Truncates on grapheme boundaries using the same measured font as drawing.
fn fit_title(title: &str, font: &FontVec, size: f32, width: f32) -> String {
    let ellipsis = "…";
    let mut fitted = String::new();
    for grapheme in title.graphemes(true) {
        let start = fitted.len();
        fitted.push_str(grapheme);
        if toolbar::text_advance(font, size, &fitted) > width {
            fitted.truncate(start);
            while !fitted.is_empty()
                && toolbar::text_advance(font, size, &format!("{fitted}{ellipsis}")) > width
            {
                let last = fitted
                    .grapheme_indices(true)
                    .next_back()
                    .map_or(0, |(i, _)| i);
                fitted.truncate(last);
            }
            if toolbar::text_advance(font, size, ellipsis) <= width {
                fitted.push_str(ellipsis);
            }
            return fitted;
        }
    }
    fitted
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn long_labels_fit_using_font_width_and_complete_graphemes() {
        let font = toolbar::load_ui_font([
            "/System/Library/Fonts/SFNS.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ])
        .unwrap();
        for scale in [1.0, 1.5, 2.0] {
            let size = 12.0 * scale;
            let width = 90.0 * scale;
            for title in [
                "A very long document title that must be truncated",
                "Cafe\u{301} 👩‍💻 browser title with many words",
            ] {
                let fitted = fit_title(title, &font, size, width);
                assert!(fitted.ends_with('…'));
                assert!(toolbar::text_advance(&font, size, &fitted) <= width);
                let prefix = fitted.strip_suffix('…').unwrap();
                assert!(title.starts_with(prefix));
                assert!(
                    title
                        .grapheme_indices(true)
                        .any(|(index, _)| index == prefix.len())
                        || prefix.len() == title.len()
                );
            }
            assert_eq!(fit_title("New Tab", &font, size, 200.0 * scale), "New Tab");
            assert_eq!(fit_title("long", &font, size, 0.0), "");
        }
    }

    #[test]
    fn close_marks_are_centered_padded_and_target_their_own_tab() {
        let ids = [TabId(0), TabId(1), TabId(2)];
        for width in [420.0, 640.0, 1120.0] {
            let strip = Strip::new(width, &ids, 0, 72.0);
            for (action, target) in &strip.regions {
                let Action::Close(id) = action else { continue };
                let tab = strip
                    .regions
                    .iter()
                    .find(|(a, _)| *a == Action::Select(*id))
                    .unwrap()
                    .1;
                let icon = close_icon_rect(*target);
                assert_eq!(icon.y() + icon.height() / 2.0, tab.y() + tab.height() / 2.0);
                assert!(icon.right() <= tab.right() - 8.0);
                assert_eq!(icon.width(), 8.0);
                assert_eq!(target.width(), 22.0);
                assert_eq!(target.height(), 24.0);
                assert!(icon.left() > tab.left());
                assert_eq!(
                    strip.action_at(Some(((icon.x() + 4.0) as f64, (icon.y() + 4.0) as f64))),
                    Some(Action::Close(*id))
                );
            }
        }
    }

    #[test]
    fn plus_hover_is_circular_and_local_in_both_themes() {
        for dark in [false, true] {
            for scale in [1.0, 1.5, 2.0] {
                let id = TabId(0);
                let mut strip = Strip::new(640.0, &[id], 0, 72.0);
                let target = strip
                    .regions
                    .iter()
                    .find(|(a, _)| *a == Action::New)
                    .unwrap()
                    .1;
                let mut idle = Pixmap::new((640.0 * scale) as u32, (100.0 * scale) as u32).unwrap();
                strip.draw(&mut idle, scale, dark, id, &[], None);
                strip.hovered = Some(Action::New);
                let mut hover = Pixmap::new(idle.width(), idle.height()).unwrap();
                strip.draw(&mut hover, scale, dark, id, &[], None);
                assert_ne!(idle.data(), hover.data());
                for y in 0..idle.height() {
                    for x in 0..idle.width() {
                        if idle.pixel(x, y) != hover.pixel(x, y) {
                            let cx = (x as f64 + 0.5) / scale;
                            let cy = (y as f64 + 0.5) / scale;
                            assert!((cx - (target.x() + 14.0) as f64).hypot(cy - 16.0) <= 13.0);
                        }
                    }
                }
                strip.hovered = None;
                strip.draw(&mut hover, scale, dark, id, &[], None);
                assert_eq!(idle.data(), hover.data());
            }
        }
    }

    #[test]
    fn plus_follows_tabs_as_they_open_close_and_overflow() {
        let ids: Vec<_> = (0..40).map(TabId).collect();
        let mut previous = 0.0;
        for count in [1, 2, 3, 40] {
            let strip = Strip::new(1120.0, &ids[..count], 0, 72.0);
            let plus = strip
                .regions
                .iter()
                .find(|(a, _)| *a == Action::New)
                .unwrap()
                .1;
            let last = strip
                .regions
                .iter()
                .rfind(|(a, _)| matches!(a, Action::Select(_)))
                .unwrap()
                .1;
            assert_eq!(plus.left(), last.right() + 7.0);
            assert_eq!(plus.width(), 28.0);
            assert_eq!(plus.y() + plus.height() / 2.0, 16.0);
            assert!(plus.right() <= 1120.0);
            assert!(plus.left() > previous);
            previous = plus.left();
        }
        let one = Strip::new(1120.0, &ids[..1], 0, 72.0);
        assert!(
            one.regions
                .iter()
                .find(|(a, _)| *a == Action::New)
                .unwrap()
                .1
                .right()
                < 400.0
        );
    }

    #[test]
    fn tabs_shrink_and_reserve_native_window_controls() {
        let ids = [TabId(0), TabId(1), TabId(2), TabId(3)];
        for inset in [0.0, 72.0] {
            let one = Strip::new(640.0, &ids[..1], 0, inset);
            let many = Strip::new(640.0, &ids, 0, inset);
            let tab_width = |strip: &Strip| {
                strip
                    .regions
                    .iter()
                    .find(|(action, _)| matches!(action, Action::Select(_)))
                    .unwrap()
                    .1
                    .width()
            };
            assert!(tab_width(&many) < tab_width(&one));
            for x in 0..inset as u32 {
                assert_eq!(many.action_at(Some((x as f64, 16.0))), None);
            }
            assert!(many.regions.iter().all(|(_, rect)| rect.left() >= inset));
            let plus = many
                .regions
                .iter()
                .find(|(a, _)| *a == Action::New)
                .unwrap()
                .1;
            assert_eq!(
                many.action_at(Some(((plus.x() + 15.0) as f64, 16.0))),
                Some(Action::New)
            );
        }
    }

    #[test]
    fn layout_bounds_overflow_and_close_targets_are_exact() {
        let ids: Vec<_> = (0..40).map(TabId).collect();
        for width in [420.0, 640.0, 1120.0] {
            for count in [1, 2, 40] {
                for offset in [0, 1, 100] {
                    let strip = Strip::new(width, &ids[..count], offset, 72.0);
                    assert!(strip.visible >= 1);
                    assert!(strip.start + strip.visible <= count);
                    for (action, rect) in &strip.regions {
                        assert!(rect.left() >= 72.0 && rect.right() <= width);
                        assert!(rect.top() >= 0.0 && rect.bottom() <= HEIGHT as f32);
                        if !matches!(action, Action::Select(_)) {
                            assert_eq!(
                                strip.action_at(Some((
                                    (rect.x() + rect.width() / 2.0) as f64,
                                    (rect.y() + rect.height() / 2.0) as f64
                                ))),
                                Some(*action)
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn active_style_changes_pixels_without_touching_the_page_or_toolbar() {
        let ids = [TabId(0), TabId(1)];
        let entries = vec![(ids[0], "Welcome".into()), (ids[1], "Example".into())];
        for dark in [false, true] {
            for scale in [1.0, 1.5, 2.0] {
                let strip = Strip::new(640.0, &ids, 0, 72.0);
                let mut a = Pixmap::new((640.0 * scale) as u32, (400.0 * scale) as u32).unwrap();
                let mut b = a.clone();
                strip.draw(&mut a, scale, dark, ids[0], &entries, None);
                strip.draw(&mut b, scale, dark, ids[1], &entries, None);
                assert_ne!(a.data(), b.data());
                let stride = a.width() as usize * 4;
                assert!(
                    a.data()[(HEIGHT * scale) as usize * stride..]
                        .iter()
                        .all(|p| *p == 0)
                );
            }
        }
    }
}
