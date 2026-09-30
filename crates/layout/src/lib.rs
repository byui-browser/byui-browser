//! Box tree construction and layout (block, inline, flex, grid).
//!
//! **Owning team**: Layout & Rendering Team
//!
//! Planned interface (architecture §3.2):
//! `layout_tree(styled_dom, viewport) -> LayoutTree`
//!
//! Layout never talks directly to the GPU.

#![forbid(unsafe_code)]

use common::ids::NodeId;

/// Width and height in CSS pixels.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

/// Axis-aligned rectangle in CSS pixels.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// Whether the point lies inside this rectangle (inclusive of the
    /// top-left edge, exclusive of the bottom-right edge).
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.width && py < self.y + self.height
    }
}

/// Placeholder for the styled DOM handed over by the CSS team.
///
/// This will move to `common` once the DOM and style shapes are fixed.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StyledDom {
    pub nodes: Vec<NodeId>,
}

/// One box in the layout tree.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutBox {
    /// `None` for the anonymous root / viewport box.
    pub node: Option<NodeId>,
    pub rect: Rect,
    pub children: Vec<LayoutBox>,
}

/// Output of [`layout_tree`].
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutTree {
    pub root: LayoutBox,
}

/// Builds the box tree and runs layout (architecture §3.2).
///
/// Currently only handles an empty document: the root box fills the viewport.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn layout_tree(styled_dom: &StyledDom, viewport: Size) -> LayoutTree {
    if styled_dom.nodes.is_empty() {
        return LayoutTree {
            root: LayoutBox {
                node: None,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: viewport.width,
                    height: viewport.height,
                },
                children: Vec::new(),
            },
        };
    }
    todo!("TODO(layout): lay out {} nodes", styled_dom.nodes.len())
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

    #[test]
    fn rect_contains_is_inclusive_top_left_exclusive_bottom_right() {
        let r = Rect {
            x: 10.0,
            y: 10.0,
            width: 5.0,
            height: 5.0,
        };
        assert!(r.contains(10.0, 10.0));
        assert!(r.contains(14.9, 14.9));
        assert!(!r.contains(15.0, 15.0));
        assert!(!r.contains(9.9, 12.0));
    }
}
