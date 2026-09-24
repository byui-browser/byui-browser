//! Conversion from layout output to paint commands.

use html::HtmlDocument;
use layout::{LayoutTree, Rect as LayoutRect};

use crate::{Color, DisplayItem, DisplayList, Rect};

const BACKGROUND: Color = Color {
    r: 210,
    g: 230,
    b: 255,
    a: 255,
};
const TEXT: Color = Color {
    r: 25,
    g: 45,
    b: 70,
    a: 255,
};

fn background_item(layout_box: &layout::LayoutBox) -> DisplayItem {
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
        color: BACKGROUND,
    }
}

/// Paints layout boxes and each node's direct text children.
///
/// Coordinates are CSS pixels and are passed to `render`, which converts them
/// to device pixels for this first software-rendered slice. Descendant text is
/// intentionally excluded here so nested elements do not paint the same text
/// more than once.
pub fn paint_document(tree: &LayoutTree, document: &HtmlDocument) -> DisplayList {
    let mut items = Vec::new();
    for layout_box in &tree.root.children {
        items.push(background_item(layout_box));
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
