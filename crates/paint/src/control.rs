//! Drawing checkbox and radio controls with display items.
//!
//! The colors follow Chrome's defaults. `accent-color` is not computed by the
//! `css` crate yet, so the checked blue is hard-coded to [`ACCENT`].

use layout::{ControlKind, ToggleControl};

use crate::{Color, CornerRadii, CornerRadius, DisplayItem, Rect};

/// The checked fill: Chrome's default accent blue `#0075ff`.
pub const ACCENT: Color = rgb(0x00, 0x75, 0xff);
/// The unchecked border: grey `#767676`.
pub const CONTROL_BORDER: Color = rgb(0x76, 0x76, 0x76);
/// The border and checked fill of disabled controls: light grey `#c5c5c5`.
pub const DISABLED: Color = rgb(0xc5, 0xc5, 0xc5);
const WHITE: Color = rgb(0xff, 0xff, 0xff);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r, g, b, a: 255 }
}

fn uniform(horizontal: f32, vertical: f32) -> CornerRadii {
    CornerRadii::from_array(
        [CornerRadius {
            horizontal,
            vertical,
        }; 4],
    )
}

fn inset(rect: Rect, by: f32) -> Rect {
    let by = by.min(rect.width / 2.0).min(rect.height / 2.0);
    Rect {
        x: rect.x + by,
        y: rect.y + by,
        width: rect.width - 2.0 * by,
        height: rect.height - 2.0 * by,
    }
}

/// An ellipse filling `rect`.
fn ellipse(rect: Rect, color: Color) -> DisplayItem {
    DisplayItem::FillRoundedRect {
        rect,
        radii: uniform(rect.width / 2.0, rect.height / 2.0),
        color,
    }
}

/// The display items that draw `control` filling `rect` (its content box,
/// in CSS pixels), back to front.
pub(crate) fn control_items(rect: Rect, control: ToggleControl) -> Vec<DisplayItem> {
    let size = rect.width.min(rect.height);
    let (border, fill) = match (control.checked, control.disabled) {
        (false, false) => (CONTROL_BORDER, WHITE),
        (false, true) => (DISABLED, WHITE),
        (true, false) => (ACCENT, ACCENT),
        (true, true) => (DISABLED, DISABLED),
    };
    match control.kind {
        ControlKind::Checkbox => {
            let radius = 2.0_f32.min(size / 2.0);
            let mut items = vec![DisplayItem::FillRoundedRect {
                rect,
                radii: uniform(radius, radius),
                color: border,
            }];
            if !control.checked {
                let inner = inset(rect, 1.0);
                let radius = (radius - 1.0).max(0.0);
                items.push(DisplayItem::FillRoundedRect {
                    rect: inner,
                    radii: uniform(radius, radius),
                    color: fill,
                });
            } else {
                // A check mark through three points, as fractions of `rect`.
                let point =
                    |fx: f32, fy: f32| (rect.x + fx * rect.width, rect.y + fy * rect.height);
                let width = (size * 0.14).max(1.0);
                let (a, b, c) = (point(0.22, 0.5), point(0.42, 0.7), point(0.78, 0.3));
                for (from, to) in [(a, b), (b, c)] {
                    items.push(DisplayItem::StrokeLine {
                        from,
                        to,
                        width,
                        color: WHITE,
                    });
                }
            }
            items
        }
        ControlKind::Radio => {
            let mut items = vec![ellipse(rect, border)];
            if control.checked {
                items.push(ellipse(inset(rect, (size * 0.15).max(1.0)), WHITE));
                items.push(ellipse(inset(rect, size * 0.3), fill));
            } else {
                items.push(ellipse(inset(rect, 1.0), fill));
            }
            items
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint_document;
    use layout::{Size, VerticalMetrics, layout_document};

    struct Metrics;

    impl layout::FontMetrics for Metrics {
        fn advance(&self, _: char, size: f32) -> f32 {
            size / 2.0
        }
        fn vertical_metrics(&self, size: f32) -> VerticalMetrics {
            VerticalMetrics {
                ascent: size * 0.8,
                descent: size * 0.2,
                line_gap: 0.0,
            }
        }
    }

    /// The display items after the canvas for one input element.
    fn paint_input(attributes: &str) -> Vec<DisplayItem> {
        let document = html::parse_raw_html(format!("<body><input {attributes}></body>"));
        let styles = css::style_document(
            &document,
            &[css::parse_stylesheet(
                "body { margin: 0 } input { margin: 0 }",
            )],
        );
        let tree = layout_document(
            &document,
            &styles,
            Size {
                width: 100.0,
                height: 100.0,
            },
            &Metrics,
        );
        let mut items = paint_document(&tree).items;
        items.remove(0);
        items
    }

    fn square(size: f32) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: size,
            height: size,
        }
    }

    #[test]
    fn unchecked_checkbox_is_a_grey_rounded_square_with_white_fill() {
        assert_eq!(
            paint_input("type=checkbox"),
            vec![
                DisplayItem::FillRoundedRect {
                    rect: square(13.0),
                    radii: uniform(2.0, 2.0),
                    color: CONTROL_BORDER,
                },
                DisplayItem::FillRoundedRect {
                    rect: inset(square(13.0), 1.0),
                    radii: uniform(1.0, 1.0),
                    color: WHITE,
                },
            ]
        );
    }

    #[test]
    fn checked_checkbox_is_blue_with_a_white_check_mark() {
        let items = paint_input("type=checkbox checked");
        assert_eq!(items.len(), 3);
        assert!(matches!(
            items[0],
            DisplayItem::FillRoundedRect { color: ACCENT, .. }
        ));
        for line in &items[1..] {
            assert!(matches!(line, DisplayItem::StrokeLine { color: WHITE, .. }));
        }
    }

    #[test]
    fn author_size_scales_the_drawn_checkbox() {
        let items = paint_input("type=checkbox checked style='width:30px;height:30px'");
        assert!(matches!(
            items[0],
            DisplayItem::FillRoundedRect { rect, .. } if rect == square(30.0)
        ));
    }

    #[test]
    fn radios_are_circles_and_checked_radios_have_a_blue_ring_and_dot() {
        assert_eq!(
            paint_input("type=radio"),
            vec![
                ellipse(square(13.0), CONTROL_BORDER),
                ellipse(inset(square(13.0), 1.0), WHITE),
            ]
        );
        let checked = paint_input("type=radio checked");
        assert_eq!(checked.len(), 3);
        assert_eq!(checked[0], ellipse(square(13.0), ACCENT));
        assert!(matches!(
            checked[1],
            DisplayItem::FillRoundedRect { color: WHITE, .. }
        ));
        assert!(matches!(
            checked[2],
            DisplayItem::FillRoundedRect { color: ACCENT, rect, .. } if rect.width < 13.0 * 0.5
        ));
    }

    #[test]
    fn disabled_controls_are_light_grey() {
        assert!(matches!(
            paint_input("type=checkbox disabled")[0],
            DisplayItem::FillRoundedRect {
                color: DISABLED,
                ..
            }
        ));
        assert!(matches!(
            paint_input("type=checkbox checked disabled")[0],
            DisplayItem::FillRoundedRect {
                color: DISABLED,
                ..
            }
        ));
    }
}
