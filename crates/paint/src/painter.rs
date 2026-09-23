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

pub fn paint_layout(tree: &LayoutTree) -> DisplayList {
    DisplayList {
        items: tree.root.children.iter().map(background_item).collect(),
    }
}

pub fn paint_document(tree: &LayoutTree, document: &HtmlDocument) -> DisplayList {
    let mut items = Vec::new();
    for layout_box in &tree.root.children {
        items.push(background_item(layout_box));
        if let Some(node) = layout_box.node {
            let html_node = html::NodeId(node.index() as usize);
            let text = document.text_content(html_node);
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
