//! Conversion from layout output to paint commands.

use html::HTMLDocument;
use layout::{LayoutTree, Rect as LayoutRect};

use crate::{Color, DisplayItem, DisplayList, Rect};

const DEFAULT_BACKGROUND: Color = Color {
    r: 242,
    g: 245,
    b: 250,
    a: 255,
};
const TEXT: Color = Color {
    r: 30,
    g: 43,
    b: 62,
    a: 255,
};

fn background_item(layout_box: &layout::LayoutBox, color: Color) -> DisplayItem {
    let LayoutRect {
        x,
        y,
        width,
        height,
    } = layout_box.rect;
    DisplayItem::FillRect {
        rect: Rect {
            x,
            y,
            width,
            height,
        },
        color,
    }
}

fn background_color(
    document: &HTMLDocument,
    node: common::ids::NodeId,
    stylesheet: &css::Stylesheet,
) -> Color {
    let html_node = layout::html_node_id(node);
    let Some(html::Node {
        kind: html::NodeKind::Element(element),
        ..
    }) = document.node(html_node)
    else {
        return DEFAULT_BACKGROUND;
    };

    stylesheet
        .rules
        .iter()
        .filter(|rule| rule.selector.0.trim() == element.name)
        .flat_map(|rule| rule.declarations.iter())
        .find_map(|declaration| {
            (declaration.property == "background-color").then(|| parse_color(&declaration.value))
        })
        .flatten()
        .unwrap_or(DEFAULT_BACKGROUND)
}

fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    let (r, g, b) = match value.to_ascii_lowercase().as_str() {
        "green" => (0, 128, 0),
        "blue" => (0, 0, 255),
        "red" => (255, 0, 0),
        "white" => (255, 255, 255),
        "black" => (0, 0, 0),
        value if value.len() == 7 && value.starts_with('#') => (
            u8::from_str_radix(&value[1..3], 16).ok()?,
            u8::from_str_radix(&value[3..5], 16).ok()?,
            u8::from_str_radix(&value[5..7], 16).ok()?,
        ),
        _ => return None,
    };
    Some(Color { r, g, b, a: 255 })
}

/// Paints layout boxes and each node's direct text children.
///
/// Coordinates are CSS pixels and are passed to `render`, which converts them
/// to device pixels for this first software-rendered slice. Descendant text is
/// intentionally excluded here so nested elements do not paint the same text
/// more than once.
pub fn paint_document(tree: &LayoutTree, document: &HTMLDocument) -> DisplayList {
    paint_document_with_stylesheet(tree, document, &css::Stylesheet::default())
}

/// Paints a document using its parsed stylesheet for element backgrounds.
///
/// Only simple element-name selectors and `background-color` declarations are
/// interpreted by this first rendering slice. Unsupported or invalid colors
/// fall back to the renderer's default background.
pub fn paint_document_with_stylesheet(
    tree: &LayoutTree,
    document: &HTMLDocument,
    stylesheet: &css::Stylesheet,
) -> DisplayList {
    let mut items = Vec::new();
    for layout_box in &tree.root.children {
        let color = layout_box
            .node
            .map(|node| background_color(document, node, stylesheet))
            .unwrap_or(DEFAULT_BACKGROUND);
        items.push(background_item(layout_box, color));
        if let Some(node) = layout_box.node {
            let html_node = layout::html_node_id(node);
            let text = document
                .node(html_node)
                .into_iter()
                .flat_map(|node| node.children.iter())
                .filter_map(|child| document.node(*child))
                .filter_map(|node| match &node.kind {
                    html::NodeKind::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>();
            if !text.is_empty() {
                items.push(DisplayItem::Text {
                    x: layout_box.rect.x + 12.0,
                    y: layout_box.rect.y + 12.0,
                    text,
                    color: TEXT,
                });
            }
        }
    }
    DisplayList { items }
}

#[cfg(test)]
mod tests {
    use super::*;
    use layout::{Size, StyledDom, layout_tree};

    #[test]
    fn direct_body_text_is_painted() {
        let document = html::parse_raw_html("<body>Hello</body>".to_owned());
        let tree = layout_tree(
            &StyledDom::from_html_document(&document),
            Size {
                width: 160.0,
                height: 120.0,
            },
        );

        let text = paint_document(&tree, &document)
            .items
            .into_iter()
            .find_map(|item| match item {
                DisplayItem::Text { text, .. } => Some(text),
                DisplayItem::FillRect { .. } => None,
            });

        assert_eq!(text.as_deref(), Some("Hello"));
    }

    #[test]
    fn stylesheet_background_color_reaches_the_display_list() {
        let document = html::parse_raw_html("<main>Welcome</main>".to_owned());
        let tree = layout_tree(
            &StyledDom::from_html_document(&document),
            Size {
                width: 160.0,
                height: 120.0,
            },
        );
        let stylesheet = css::parse_stylesheet("main { background-color: green; }");

        assert_eq!(
            paint_document_with_stylesheet(&tree, &document, &stylesheet).items[0],
            DisplayItem::FillRect {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 160.0,
                    height: 100.0,
                },
                color: Color {
                    r: 0,
                    g: 128,
                    b: 0,
                    a: 255,
                },
            }
        );
    }
}
