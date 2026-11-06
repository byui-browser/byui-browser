//! The layout entry point.

use html::{HTMLDocument, NodeID, NodeKind};

use crate::box_tree::{StyleMap, build_initial_containing_block, root_element};
use crate::{FontMetrics, LayoutBox, LayoutTree, Size};

/// State shared by the layout algorithms.
pub(crate) struct Context<'a> {
    pub fonts: &'a dyn FontMetrics,
}

/// Lays out `document` for a viewport of `viewport` CSS pixels.
///
/// `styles` comes from [`css::style_document`] for the same document, and
/// `fonts` must measure text the way the renderer will draw it.
///
/// Implemented (CSS 2.2 visual formatting model, with CSS Display 3 box
/// generation):
/// - Box generation from computed `display`: `none`, `contents`, `block`,
///   `inline`, `inline-block`, and `flow-root`. The root element is
///   blockified; inline runs beside blocks get anonymous block boxes; a block
///   inside an inline splits the inline.
/// - Block layout: `width`/`height`, `auto` margins and over-constraint
///   (§10.3.3), percentages, padding, and vertical margin collapsing between
///   siblings, parents and children, and through empty boxes (§8.3.1).
/// - Inline layout: white-space collapsing, wrapping at spaces, `<br>`, line
///   boxes sized by baseline alignment, and inline-blocks with shrink-to-fit
///   widths.
/// - Canvas background propagation from the root element or `body`
///   (CSS Backgrounds 3 §2.11.2).
///
/// `display:flex` and `display:inline-flex` use a single-line flex subset with
/// `flex-direction:row|column`, `flex-grow`, and `gap`. Wrapping, shrinking,
/// alignment, and anonymous flex items for mixed text are not implemented.
/// Grid and table modes are preserved distinctly in computed display values,
/// but still use ordinary flow layout. Other supported properties are
/// margins, padding, `width`, `height`, `font-size`, `color`, and
/// `background-color`; unsupported properties use their initial values.
/// Floats, positioning, borders, and min/max sizes are not implemented.
/// Content may overflow the viewport; the root box's rect stays viewport-sized.
pub fn layout_document(
    document: &HTMLDocument,
    styles: &css::ComputedStyles,
    viewport: Size,
    fonts: &dyn FontMetrics,
) -> LayoutTree {
    log::debug!(
        "layout pass start: viewport {}x{}",
        viewport.width,
        viewport.height
    );
    let root = root_element(document);
    if root.is_none() {
        log::debug!("document has no root element; laying out an empty canvas");
    }
    let style_map = StyleMap::new(document, styles, root, viewport);
    let icb = build_initial_containing_block(document, &style_map);
    let context = Context { fonts };
    let mut tree = LayoutTree {
        root: context.layout_initial_containing_block(&icb, viewport),
    };
    propagate_canvas_background(&mut tree, document, &style_map, root);
    if log::log_enabled!(log::Level::Debug) {
        log::debug!(
            "layout pass finished: {} boxes, content height {}",
            count_boxes(&tree.root),
            content_height(&tree.root)
        );
    }
    tree
}

/// Number of boxes in the subtree rooted at `layout_box`.
fn count_boxes(layout_box: &LayoutBox) -> usize {
    1 + layout_box.children.iter().map(count_boxes).sum::<usize>()
}

/// Bottom edge of the lowest box or line in the subtree, relative to the
/// subtree root's origin.
fn content_height(layout_box: &LayoutBox) -> f32 {
    let lines = layout_box
        .lines
        .iter()
        .map(|line| layout_box.rect.y + line.rect.bottom());
    let children = layout_box
        .children
        .iter()
        .map(|child| layout_box.rect.y + content_height(child));
    lines.chain(children).fold(layout_box.rect.height, f32::max)
}

/// Moves the root element's background to the canvas, or for HTML documents
/// `body`'s background when the root's is transparent. The element whose
/// background propagates no longer paints it itself.
fn propagate_canvas_background(
    tree: &mut LayoutTree,
    document: &HTMLDocument,
    styles: &StyleMap,
    root: Option<NodeID>,
) {
    let Some(root) = root else {
        return;
    };
    let background = |node: NodeID| styles.get(node).and_then(|style| style.background_color);
    let is_element = |node: NodeID, name: &str| {
        matches!(
            document.node(node).map(|node| node.get_kind()),
            Some(NodeKind::Element(element)) if element.get_name() == name
        )
    };
    let source = if background(root).is_some() {
        Some(root)
    } else if is_element(root, "html") {
        document
            .node(root)
            .into_iter()
            .flat_map(|node| node.get_children().iter().copied())
            .find(|child| is_element(*child, "body"))
            .filter(|body| background(*body).is_some())
    } else {
        None
    };
    let Some(source) = source else {
        return;
    };
    log::trace!("canvas background propagated from node {source:?}");
    tree.root.style.background_color = background(source);
    if let Some(layout_box) = find_box_mut(&mut tree.root, source) {
        layout_box.style.background_color = None;
    }
}

fn find_box_mut(layout_box: &mut LayoutBox, node: NodeID) -> Option<&mut LayoutBox> {
    if layout_box.node == Some(node) {
        return Some(layout_box);
    }
    layout_box
        .children
        .iter_mut()
        .find_map(|child| find_box_mut(child, node))
}
