//! Conversion from layout output to paint commands.

use layout::{BoxKind, LayoutTree, PaintItem};

use crate::{Color, DisplayItem, DisplayList};

/// The user agent's default canvas color: the `Canvas` system color in a light
/// color scheme. It is used when neither the root element nor `body` has a
/// background to propagate to the canvas.
pub const CANVAS_COLOR: Color = Color {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

/// Paints a layout tree into a display list.
///
/// Items follow the order from [`LayoutTree::paint_order`], which implements
/// CSS 2.2 Appendix E: the canvas first, then block backgrounds in tree order,
/// then the inline content of each line box (inline box backgrounds and text).
/// The canvas covers the viewport with the propagated root background, or
/// [`CANVAS_COLOR`]. Text is drawn at its layout baseline and font size.
///
/// Only backgrounds and text are painted; borders, outlines, and other
/// decorations are not supported yet.
pub fn paint_document(tree: &LayoutTree) -> DisplayList {
    let mut items = Vec::new();
    for item in tree.paint_order() {
        match item {
            PaintItem::BoxDecorations(layout_box) => {
                let color = match (layout_box.kind, layout_box.style.background_color) {
                    (BoxKind::Viewport, background) => {
                        Some(background.map_or(CANVAS_COLOR, Color::from))
                    }
                    (_, background) => background.map(Color::from),
                };
                if let Some(color) = color {
                    items.push(DisplayItem::FillRect {
                        rect: layout_box.rect,
                        color,
                    });
                }
            }
            PaintItem::InlineBox(inline) => {
                if let Some(color) = inline.style.background_color {
                    items.push(DisplayItem::FillRect {
                        rect: inline.rect,
                        color: color.into(),
                    });
                }
            }
            PaintItem::Text(text) => {
                if !text.text.trim().is_empty() {
                    items.push(DisplayItem::Text {
                        x: text.rect.x,
                        baseline: text.baseline,
                        font_size: text.font_size,
                        text: text.text.clone(),
                        color: text.color.into(),
                    });
                }
            }
        }
    }
    DisplayList { items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::{FontMetrics, Size, VerticalMetrics, layout_document};

    struct TestFont;

    impl FontMetrics for TestFont {
        fn advance(&self, _character: char, font_size: f32) -> f32 {
            font_size / 2.0
        }

        fn vertical_metrics(&self, font_size: f32) -> VerticalMetrics {
            VerticalMetrics {
                ascent: font_size * 0.8,
                descent: font_size * 0.2,
                line_gap: font_size * 0.2,
            }
        }
    }

    fn paint(html: &str, css: &str) -> DisplayList {
        let document = html::parse_raw_html(html.to_owned());
        let styles = css::style_document(&document, &[css::parse_stylesheet(css)]);
        let tree = layout_document(
            &document,
            &styles,
            Size {
                width: 160.0,
                height: 120.0,
            },
            &TestFont,
        );
        paint_document(&tree)
    }

    #[test]
    fn the_canvas_is_painted_first_with_the_default_canvas_color() {
        let list = paint("<body>Hello</body>", "");
        assert_eq!(
            list.items[0],
            DisplayItem::FillRect {
                rect: layout::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 160.0,
                    height: 120.0
                },
                color: CANVAS_COLOR,
            }
        );
    }

    #[test]
    fn text_is_painted_at_its_baseline_after_all_block_backgrounds() {
        let list = paint(
            "<body><div>Hello</div><p>World</p></body>",
            "body { margin: 0 } div { background-color: red } p { margin: 0; background-color: blue }",
        );
        let summary = list
            .items
            .iter()
            .map(|item| match item {
                DisplayItem::FillRect { color, .. } => format!("fill {}", color.b),
                DisplayItem::Text { text, baseline, .. } => format!("text {text} {baseline:.1}"),
            })
            .collect::<Vec<_>>();
        // Canvas, then both block backgrounds, then the text of both blocks
        // (CSS 2.2 Appendix E). Baselines are 14.4px below each line top.
        assert_eq!(
            summary,
            [
                "fill 255",
                "fill 0",
                "fill 255",
                "text Hello 14.4",
                "text World 33.6"
            ]
        );
    }
}
