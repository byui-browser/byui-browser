//! Block layout orchestration.

use crate::{LayoutBox, LayoutTree, Rect, Size, StyledDom};

/// Builds a layout tree for a CSS-pixel viewport.
///
/// The first slice treats every element as a block and gives it a fixed
/// 100 CSS-pixel height. Nesting and computed styles are not interpreted yet;
/// every element is represented as a direct child of the root.
pub fn layout_tree(styled_dom: &StyledDom, viewport: Size) -> LayoutTree {
    let block_height = 100.0;
    let children = styled_dom
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| LayoutBox {
            node: Some(*node),
            rect: Rect {
                x: 0.0,
                y: index as f32 * block_height,
                width: viewport.width,
                height: block_height,
            },
            children: Vec::new(),
        })
        .collect();

    LayoutTree {
        root: LayoutBox {
            node: None,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: viewport.width,
                height: viewport.height,
            },
            children,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_document_root_fills_viewport() {
        let viewport = Size {
            width: 800.0,
            height: 600.0,
        };
        let tree = layout_tree(&StyledDom::default(), viewport);
        assert_eq!(tree.root.rect.width, 800.0);
        assert_eq!(tree.root.rect.height, 600.0);
        assert!(tree.root.children.is_empty());
    }
}
