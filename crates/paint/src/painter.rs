//! Conversion from layout output to paint commands.

use layout::{BoxKind, LayoutTree, PaintItem};

use crate::{Color, DisplayItem, DisplayList, PaintBox};

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
/// Backgrounds are clipped to the rounded border box when `border-radius` is
/// set, producing [`DisplayItem::FillRoundedRect`]. Each outer `box-shadow`
/// produces a [`DisplayItem::BoxShadow`] just before its box's background,
/// last shadow first so the first one ends up on top. Shadows are not
/// clipped out beneath the box, so they show through a translucent
/// background, and inset shadows are not painted yet. An inline box split across
/// lines rounds every fragment's corners, as if each fragment were a whole
/// box. Borders, outlines, and other decorations are not supported yet.
///
/// Replaced boxes (`<img>`, inline `<svg>`) produce a [`DisplayItem::Image`]
/// covering their content box, after their background. Their pictures are
/// not clipped to `border-radius`. Checkbox and radio inputs instead draw
/// their control in the content box with [`DisplayItem::FillRoundedRect`]
/// and, for a check mark, [`DisplayItem::StrokeLine`]; see [`crate::ACCENT`].
pub fn paint_document(tree: &LayoutTree) -> DisplayList {
    paint(tree, 0.0, false, |_| Vec::new())
}

/// Paints the part of a layout tree visible when its viewport is scrolled
/// down by `scroll_y` CSS pixels.
///
/// Coordinates in the result are relative to the viewport: content moves up
/// by `scroll_y`, while the canvas still covers the whole viewport, because
/// the canvas is infinite. Items that lie entirely above or below the
/// viewport are left out, so long documents paint only what is on screen.
/// Clamp `scroll_y` to `0.0..=tree.scroll_height() - viewport height`;
/// other values paint blank canvas past the content.
///
/// Only vertical scrolling is supported. With `scroll_y` of `0.0` this
/// matches [`paint_document`] minus the culled off-screen items.
pub fn paint_document_scrolled(tree: &LayoutTree, scroll_y: f32) -> DisplayList {
    paint(tree, scroll_y, true, |_| Vec::new())
}

/// Paints a scrolled document with optional backgrounds immediately before each text run.
///
/// `backgrounds` receives visible layout text fragments in document
/// coordinates. It returns colored rectangles in document CSS pixels; callers own
/// range measurement and color policy. Rectangles follow the same scroll transform
/// and viewport clipping as ordinary paint commands. Return an empty vector for an
/// unchanged run. This does not alter layout, hit testing, or glyph rendering.
pub fn paint_document_scrolled_with_text_backgrounds(
    tree: &LayoutTree,
    scroll_y: f32,
    backgrounds: impl FnMut(&layout::TextFragment) -> Vec<PaintBox>,
) -> DisplayList {
    paint(tree, scroll_y, true, backgrounds)
}

fn paint(
    tree: &LayoutTree,
    scroll_y: f32,
    cull: bool,
    mut backgrounds: impl FnMut(&layout::TextFragment) -> Vec<PaintBox>,
) -> DisplayList {
    log::trace!("building display list (scroll_y {scroll_y}, cull {cull})");
    let viewport = tree.root.rect;
    // Whether a content rect in document coordinates is (partly) on screen.
    let visible = |rect: &layout::Rect| {
        !cull || (rect.bottom() - scroll_y > viewport.y && rect.y - scroll_y < viewport.bottom())
    };
    let scrolled = |rect: layout::Rect| rect.translate(0.0, -scroll_y);
    let mut items = Vec::new();
    for item in tree.paint_order() {
        match item {
            PaintItem::BoxDecorations(layout_box) => match layout_box.kind {
                // The canvas is infinite, so it is never rounded.
                BoxKind::Viewport => items.push(DisplayItem::FillRect {
                    rect: viewport,
                    color: layout_box
                        .style
                        .background_color
                        .map_or(CANVAS_COLOR, Color::from),
                }),
                _ => {
                    for shadow in shadows(layout_box.rect, &layout_box.style) {
                        if visible(&shadow.bounds()) {
                            items.push(shadow.item(scroll_y));
                        }
                    }
                    if let Some(color) = layout_box.style.background_color
                        && visible(&layout_box.rect)
                    {
                        items.push(background(
                            scrolled(layout_box.rect),
                            &layout_box.style,
                            color.into(),
                        ));
                    }
                    if visible(&layout_box.rect) {
                        items.extend(borders(scrolled(layout_box.rect), &layout_box.style));
                    }
                }
            },
            PaintItem::InlineBox(inline) => {
                for shadow in shadows(inline.rect, &inline.style) {
                    if visible(&shadow.bounds()) {
                        items.push(shadow.item(scroll_y));
                    }
                }
                if let Some(color) = inline.style.background_color
                    && visible(&inline.rect)
                {
                    items.push(background(
                        scrolled(inline.rect),
                        &inline.style,
                        color.into(),
                    ));
                }
                if visible(&inline.rect) {
                    items.extend(borders(scrolled(inline.rect), &inline.style));
                }
            }
            PaintItem::Text(text) => {
                if visible(&text.rect) {
                    for background in backgrounds(text) {
                        if let Some(color) = background.background {
                            items.push(DisplayItem::FillRect {
                                rect: scrolled(background.rect),
                                color,
                            });
                        }
                    }
                }
                if !text.text.trim().is_empty() && visible(&text.rect) {
                    items.push(DisplayItem::Text {
                        x: text.rect.x,
                        baseline: text.baseline - scroll_y,
                        font_size: text.font_size,
                        font: text.font,
                        text: text.text.clone(),
                        color: text.color.into(),
                        clip: text.clip.map(scrolled),
                    });
                }
            }
            PaintItem::Image(layout_box) => {
                let rect = layout_box.content_rect();
                if let Some(image) = &layout_box.image
                    && rect.width > 0.0
                    && rect.height > 0.0
                    && visible(&rect)
                {
                    items.push(DisplayItem::Image {
                        rect: scrolled(rect),
                        image: image.clone(),
                    });
                }
                if let Some(control) = layout_box.control
                    && rect.width > 0.0
                    && rect.height > 0.0
                    && visible(&rect)
                {
                    items.extend(crate::control::control_items(scrolled(rect), control));
                }
            }
        }
    }
    log::debug!("display list built: {} items", items.len());
    if log::log_enabled!(log::Level::Trace) {
        for item in &items {
            log::trace!("display item: {item:?}");
        }
    }
    DisplayList { items }
}

/// One outer shadow's shape, before scrolling.
struct Shadow {
    rect: layout::Rect,
    radii: crate::CornerRadii,
    blur: f32,
    color: Color,
}

impl Shadow {
    /// The area the shadow can paint, blur included: the renderer fades the
    /// edge over three standard deviations (`1.5 * blur`) plus a pixel.
    fn bounds(&self) -> layout::Rect {
        let reach = 1.5 * self.blur + 1.0;
        layout::Rect {
            x: self.rect.x - reach,
            y: self.rect.y - reach,
            width: self.rect.width + 2.0 * reach,
            height: self.rect.height + 2.0 * reach,
        }
    }

    /// The display item, moved up by `scroll_y`.
    fn item(self, scroll_y: f32) -> DisplayItem {
        DisplayItem::BoxShadow {
            rect: self.rect.translate(0.0, -scroll_y),
            radii: self.radii,
            blur: self.blur,
            color: self.color,
        }
    }
}

/// The outer shadows of a box with border box `rect`, in paint order: the
/// last shadow in the list first. Inset shadows, fully transparent ones,
/// and ones a negative spread shrinks to nothing are skipped.
fn shadows(rect: layout::Rect, style: &layout::BoxStyle) -> Vec<Shadow> {
    let radii = style.border_radius.resolve(rect);
    style
        .box_shadow
        .iter()
        .rev()
        .filter(|shadow| !shadow.inset && shadow.color.a > 0)
        .filter_map(|shadow| {
            let spread = shadow.spread;
            let shape = layout::Rect {
                x: rect.x + shadow.offset_x - spread,
                y: rect.y + shadow.offset_y - spread,
                width: rect.width + 2.0 * spread,
                height: rect.height + 2.0 * spread,
            };
            if shape.width <= 0.0 || shape.height <= 0.0 {
                return None;
            }
            // Rounded corners grow or shrink with the spread; square ones
            // stay square (CSS Backgrounds 3 §7.1.1).
            let grow = |radius: f32| {
                if radius > 0.0 {
                    (radius + spread).max(0.0)
                } else {
                    0.0
                }
            };
            let radii = layout::Corners::from_array(radii.into_array().map(|corner| {
                layout::CornerRadius {
                    horizontal: grow(corner.horizontal),
                    vertical: grow(corner.vertical),
                }
            }));
            Some(Shadow {
                rect: shape,
                radii,
                blur: shadow.blur,
                color: shadow.color.into(),
            })
        })
        .collect()
}

// Square border strips. Patterns are anchored to the outer edge of each side.
fn borders(rect: layout::Rect, style: &layout::BoxStyle) -> Vec<DisplayItem> {
    let w = style.border_width;
    let mut items = Vec::new();
    for (r, color, line, horizontal) in [
        (
            layout::Rect {
                height: w.top.min(rect.height),
                ..rect
            },
            style.border_color.top,
            &style.border_style.top,
            true,
        ),
        (
            layout::Rect {
                x: rect.right() - w.right.min(rect.width),
                width: w.right.min(rect.width),
                ..rect
            },
            style.border_color.right,
            &style.border_style.right,
            false,
        ),
        (
            layout::Rect {
                y: rect.bottom() - w.bottom.min(rect.height),
                height: w.bottom.min(rect.height),
                ..rect
            },
            style.border_color.bottom,
            &style.border_style.bottom,
            true,
        ),
        (
            layout::Rect {
                width: w.left.min(rect.width),
                ..rect
            },
            style.border_color.left,
            &style.border_style.left,
            false,
        ),
    ] {
        let thickness = if horizontal { r.height } else { r.width };
        let length = if horizontal { r.width } else { r.height };
        if thickness <= 0.0 || length <= 0.0 {
            continue;
        }
        let mut fill = |rect| {
            items.push(DisplayItem::FillRect {
                rect,
                color: color.into(),
            })
        };
        if line == "dashed" || line == "dotted" {
            let dash = thickness * if line == "dashed" { 3.0 } else { 1.0 };
            let mut offset = 0.0;
            while offset < length {
                let size = dash.min(length - offset);
                fill(if horizontal {
                    layout::Rect {
                        x: r.x + offset,
                        width: size,
                        ..r
                    }
                } else {
                    layout::Rect {
                        y: r.y + offset,
                        height: size,
                        ..r
                    }
                });
                offset += dash * 2.0;
            }
        } else if line == "double" && thickness >= 3.0 {
            let third = thickness / 3.0;
            for offset in [0.0, thickness - third] {
                fill(if horizontal {
                    layout::Rect {
                        y: r.y + offset,
                        height: third,
                        ..r
                    }
                } else {
                    layout::Rect {
                        x: r.x + offset,
                        width: third,
                        ..r
                    }
                });
            }
        } else {
            fill(r);
        }
    }
    items
}

/// The background fill for a box's border box `rect`, rounded by its
/// `border-radius` when that is non-zero.
fn background(rect: layout::Rect, style: &layout::BoxStyle, color: Color) -> DisplayItem {
    let radii = style.border_radius.resolve(rect);
    if radii.is_zero() {
        DisplayItem::FillRect { rect, color }
    } else {
        DisplayItem::FillRoundedRect { rect, radii, color }
    }
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
    fn patterned_and_partial_borders_paint_supported_geometry() {
        for (line, expected) in [
            ("solid", 4),
            ("inset", 4),
            ("double", 8),
            ("dashed", 8),
            ("dotted", 24),
        ] {
            let list = paint(
                "<div></div>",
                &format!(
                    "body {{ margin: 0 }} div {{ width: 30px; height: 30px; border: 3px {line} red; border-radius: 8px }}"
                ),
            );
            let strips = list.items.iter().filter(|item| matches!(item, DisplayItem::FillRect { color, .. } if *color == Color::from(css::parse_color("red").unwrap()))).count();
            assert_eq!(strips, expected, "{line}");
        }
        let list = paint(
            "<div></div>",
            "div { border: 4px solid red; border-style: none }",
        );
        assert!(!list.items.iter().any(|item| matches!(item, DisplayItem::FillRect { color, .. } if *color == Color::from(css::parse_color("red").unwrap()))));
    }

    #[test]
    fn box_shadows_paint_under_the_background_last_shadow_first() {
        let list = paint(
            "<body><div>x</div></body>",
            "body { margin: 0 } div { width: 40px; height: 20px; margin: 10px; \
             background-color: white; border-radius: 4px; \
             box-shadow: 2px 3px 6px 1px #ff0000, inset 0 0 2px #00ff00, 0 0 0 0 rgba(0, 0, 255, 0), \
             -1px 0 #0000ff }",
        );
        let shadows: Vec<_> = list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::BoxShadow {
                    rect,
                    radii,
                    blur,
                    color,
                } => Some((*rect, *radii, *blur, *color)),
                _ => None,
            })
            .collect();
        // The inset and the fully transparent shadows are skipped.
        assert_eq!(shadows.len(), 2);
        let (rect, radii, blur, color) = shadows[0];
        assert_eq!(color.b, 255);
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (9.0, 10.0, 40.0, 20.0)
        );
        assert_eq!((radii.top_left.horizontal, blur), (4.0, 0.0));
        let (rect, radii, blur, color) = shadows[1];
        assert_eq!(color.r, 255);
        // Offset by (2, 3) and grown by the 1px spread on every side.
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (11.0, 12.0, 42.0, 22.0)
        );
        assert_eq!((radii.top_left.horizontal, blur), (5.0, 6.0));
        // Both shadows come right before the background they belong to.
        let background = list
            .items
            .iter()
            .position(|item| matches!(item, DisplayItem::FillRoundedRect { .. }))
            .unwrap();
        assert!(matches!(
            list.items[background - 1],
            DisplayItem::BoxShadow { .. }
        ));
        assert!(matches!(
            list.items[background - 2],
            DisplayItem::BoxShadow { .. }
        ));
    }

    #[test]
    fn a_shrinking_spread_can_remove_a_shadow() {
        let list = paint(
            "<body><div>x</div></body>",
            "body { margin: 0 } div { width: 10px; height: 10px; box-shadow: 0 0 0 -6px red }",
        );
        assert!(
            !list
                .items
                .iter()
                .any(|item| matches!(item, DisplayItem::BoxShadow { .. }))
        );
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
                DisplayItem::FillRoundedRect { color, .. } => format!("round {}", color.b),
                DisplayItem::Text { text, baseline, .. } => format!("text {text} {baseline:.1}"),
                DisplayItem::Image { .. } => "image".to_owned(),
                DisplayItem::BoxShadow { .. } => "shadow".to_owned(),
                DisplayItem::StrokeLine { .. } => "line".to_owned(),
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

    #[test]
    fn input_values_are_painted_clipped_to_the_control() {
        let list = paint(
            "<body><input style='width: 40px; padding: 0 2px; font-size: 16px' value='much too long'></body>",
            "body { margin: 0 }",
        );
        let (text, clip) = list
            .items
            .iter()
            .find_map(|item| match item {
                DisplayItem::Text { text, clip, .. } => Some((text.as_str(), *clip)),
                _ => None,
            })
            .expect("the value is painted");
        assert_eq!(text, "much too long");
        // The padding box: inside the input's 2px user-agent border.
        let clip = clip.expect("control text is clipped");
        assert_eq!((clip.x, clip.width), (2.0, 44.0));
    }

    #[test]
    fn border_radius_rounds_the_background_and_fits_the_box() {
        let list = paint(
            "<body><div>Hi</div><p>Square</p></body>",
            "body { margin: 0 } div { height: 30px; background-color: red; border-radius: 3px 40px } \
             p { margin: 0; background-color: blue; border-radius: 0 }",
        );
        let DisplayItem::FillRoundedRect { rect, radii, color } = list.items[1] else {
            panic!("expected a rounded background, got {:?}", list.items[1]);
        };
        assert_eq!((rect.width, rect.height, color.r), (160.0, 30.0, 255));
        // 40px + 40px exceeds the 30px height, so radii scale by 30 / 43.
        let scale = 30.0 / 43.0;
        assert!((radii.top_left.horizontal - 3.0 * scale).abs() < 1e-4);
        assert!((radii.top_right.vertical - 40.0 * scale).abs() < 1e-4);
        assert!(matches!(list.items[2], DisplayItem::FillRect { .. }));
    }

    #[test]
    fn scrolling_moves_content_up_and_keeps_the_canvas_on_screen() {
        let document = html::parse_raw_html(
            "<body><div id=\"a\">Top</div><div id=\"b\">Bottom</div></body>".to_owned(),
        );
        let styles = css::style_document(
            &document,
            &[css::parse_stylesheet(
                "body { margin: 0 } div { height: 100px; background-color: red }",
            )],
        );
        let tree = layout_document(
            &document,
            &styles,
            Size {
                width: 160.0,
                height: 120.0,
            },
            &TestFont,
        );

        let top = paint_document_scrolled(&tree, 0.0);
        assert_eq!(top.items, paint_document(&tree).items);

        let list = paint_document_scrolled(&tree, 100.0);
        // The canvas stays at the viewport; only the second div is visible.
        assert_eq!(
            list.items[0],
            DisplayItem::FillRect {
                rect: tree.root.rect,
                color: CANVAS_COLOR,
            }
        );
        let fills: Vec<_> = list.items[1..]
            .iter()
            .filter_map(|item| match item {
                DisplayItem::FillRect { rect, .. } => Some(rect.y),
                _ => None,
            })
            .collect();
        assert_eq!(fills, [0.0]);
        let texts: Vec<_> = list
            .items
            .iter()
            .filter_map(|item| match item {
                DisplayItem::Text { text, baseline, .. } => Some((text.as_str(), *baseline)),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].0, "Bottom");
        assert!((texts[0].1 - 14.4).abs() < 0.01);
    }
}

#[cfg(test)]
mod text_background_tests {
    use super::*;
    use layout::{FontMetrics, Size, layout_document};
    use render_test_metrics::Metrics;
    mod render_test_metrics {
        pub struct Metrics;
        impl layout::FontMetrics for Metrics {
            fn advance(&self, c: char, size: f32) -> f32 {
                if c == 'W' { size } else { size / 4.0 }
            }
            fn vertical_metrics(&self, size: f32) -> layout::VerticalMetrics {
                layout::VerticalMetrics {
                    ascent: size * 0.8,
                    descent: size * 0.2,
                    line_gap: 0.0,
                }
            }
        }
    }
    fn tree() -> LayoutTree {
        let doc = html::parse_raw_html("<body><p>WiWi</p><p>lower</p></body>".into());
        let styles = css::style_document(&doc, &[]);
        layout_document(
            &doc,
            &styles,
            Size {
                width: 200.0,
                height: 100.0,
            },
            &Metrics,
        )
    }
    #[test]
    fn empty_text_background_hook_preserves_normal_output_exactly() {
        let tree = tree();
        for scroll in [0.0, 12.5] {
            assert_eq!(
                paint_document_scrolled(&tree, scroll),
                paint_document_scrolled_with_text_backgrounds(&tree, scroll, |_| Vec::new())
            );
        }
    }
    #[test]
    fn backgrounds_are_before_their_text_and_follow_scroll_coordinates() {
        let tree = tree();
        let color = Color {
            r: 255,
            g: 220,
            b: 0,
            a: 140,
        };
        let scroll = 3.0;
        let list = paint_document_scrolled_with_text_backgrounds(&tree, scroll, |text| {
            vec![PaintBox {
                rect: layout::Rect {
                    x: text.rect.x + Metrics.text_width("W", text.font_size),
                    y: text.rect.y,
                    width: Metrics.text_width("i", text.font_size),
                    height: text.rect.height,
                },
                background: Some(color),
            }]
        });
        let original = tree
            .paint_order()
            .into_iter()
            .filter_map(|item| {
                if let PaintItem::Text(text) = item {
                    Some(text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (index, item) in list.items.iter().enumerate() {
            if let DisplayItem::Text { text, baseline, .. } = item {
                let source = original.iter().find(|t| t.text == *text).unwrap();
                assert_eq!(*baseline, source.baseline - scroll);
                let DisplayItem::FillRect {
                    rect,
                    color: actual,
                } = list.items[index - 1]
                else {
                    panic!("missing text background")
                };
                assert_eq!(actual, color);
                assert_eq!(rect.y, source.rect.y - scroll);
                assert_eq!(rect.width, Metrics.text_width("i", source.font_size));
                assert!(rect.width < source.rect.width);
            }
        }
    }
    #[test]
    fn offscreen_fragments_do_not_request_backgrounds() {
        let tree = tree();
        let mut count = 0;
        paint_document_scrolled_with_text_backgrounds(&tree, 10000.0, |_| {
            count += 1;
            vec![]
        });
        assert_eq!(count, 0);
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use layout::{ImageKey, ImageMetrics, Size, layout_document_with_images};

    struct Metrics;

    impl layout::FontMetrics for Metrics {
        fn advance(&self, _: char, size: f32) -> f32 {
            size / 2.0
        }
        fn vertical_metrics(&self, size: f32) -> layout::VerticalMetrics {
            layout::VerticalMetrics {
                ascent: size * 0.8,
                descent: size * 0.2,
                line_gap: 0.0,
            }
        }
    }

    struct Images;

    impl ImageMetrics for Images {
        fn natural_size(&self, _: &ImageKey) -> Option<Size> {
            Some(Size {
                width: 30.0,
                height: 10.0,
            })
        }
    }

    #[test]
    fn replaced_boxes_paint_their_picture_in_the_content_box_after_the_background() {
        let document = html::parse_raw_html(
            "<body><img src=a.png><div style=height:300px></div><img src=b.png></body>".into(),
        );
        let styles = css::style_document(
            &document,
            &[css::parse_stylesheet(
                "body { margin: 0 } img { display: block; padding: 2px; background-color: red }",
            )],
        );
        let tree = layout_document_with_images(
            &document,
            &styles,
            Size {
                width: 200.0,
                height: 100.0,
            },
            &Metrics,
            &Images,
        );
        let list = paint_document_scrolled(&tree, 0.0);
        let background = list
            .items
            .iter()
            .position(|item| matches!(item, DisplayItem::FillRect { color, .. } if color.g == 0))
            .expect("image background");
        let image = list
            .items
            .iter()
            .position(|item| matches!(item, DisplayItem::Image { .. }))
            .expect("image item");
        assert!(image > background);
        assert_eq!(
            list.items[image],
            DisplayItem::Image {
                rect: layout::Rect {
                    x: 2.0,
                    y: 2.0,
                    width: 30.0,
                    height: 10.0
                },
                image: ImageKey::Src("a.png".into()),
            }
        );
        // The second image is far below the viewport and culled.
        let images = list
            .items
            .iter()
            .filter(|item| matches!(item, DisplayItem::Image { .. }))
            .count();
        assert_eq!(images, 1);
    }
}
