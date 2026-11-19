//! The layout entry point.

use html::{HTMLDocument, NodeID, NodeKind};

use crate::box_tree::{StyleMap, build_initial_containing_block, root_element};
use crate::{FontMetrics, ImageMetrics, LayoutBox, LayoutTree, NoImages, Size};

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
/// `flex-direction:row|column`, `flex-basis`, `flex-grow`, `flex-shrink`, and
/// `gap`. Main-axis free-space distribution and `align-items` start, center,
/// and end placement are supported for definite cross sizes. Wrapping, full
/// baseline/stretch alignment, and anonymous flex items for mixed text are
/// not implemented.
/// Grid modes are preserved distinctly in computed display values, but still
/// use ordinary flow layout. Tables lay out direct children in one row; full
/// row groups, column sizing, and collapsed borders are not implemented.
/// Non-replaced boxes support `width: min-content`, `max-content`, and
/// `fit-content`; intrinsic sizing keywords on `height` and replaced boxes
/// are not implemented. Other supported properties are margins, padding,
/// `min-width`, `width`, `height`, `font-size`, `font-family`, `font-weight`,
/// `font-style`, `color`, `background-color`, and the color component of the
/// `background` shorthand. `text-decoration` paints simple text lines,
/// `outline` paints a square outline without changing layout, and
/// `vertical-align` applies an approximate baseline shift to inline boxes;
/// unsupported properties (including the `font` shorthand) use their initial
/// values. The font properties choose a face through
/// [`FontMetrics::select_font`], which measures each text run.
/// - Floats (`float: left|right`, CSS 2.2 §9.5): shrink-to-fit boxes placed
///   as high as possible and stacked beside or below earlier floats; line
///   boxes in the same block formatting context shorten beside them, and
///   formatting context roots grow to contain them (§10.6.7). `clear` is not
///   supported, and in-flow formatting context roots do not avoid floats.
/// - Positioning (`position` with `top`, `right`, `bottom`, `left`, and the
///   `inset` shorthand, CSS 2.2 §9.3 and §10.3.7/§10.6.4): `relative` shifts
///   a box after layout; `absolute` places it against the padding box of the
///   nearest positioned ancestor block box (or the viewport), falling back to
///   its static position for `auto` insets; `fixed` places it against the
///   viewport. `sticky` stays at its in-flow position. `z-index` is not
///   supported; see [`LayoutTree::paint_order`] for stacking.
///
/// Border widths, colors, and styles paint on boxes. Min/max sizes apply to
/// replaced elements; ordinary blocks and flex items also honor `min-width`.
/// Other `min-*`/`max-*` constraints are incomplete.
/// Background images and full background layers, opacity groups, and
/// standards-complete `vertical-align` are not implemented.
/// Content may overflow the viewport; the root box's rect stays viewport-sized.
///
/// This lays out without images: every `<img>` is treated as broken. Use
/// [`layout_document_with_images`] to size images by their natural size.
pub fn layout_document(
    document: &HTMLDocument,
    styles: &css::ComputedStyles,
    viewport: Size,
    fonts: &dyn FontMetrics,
) -> LayoutTree {
    layout_document_with_images(document, styles, viewport, fonts, &NoImages)
}

/// Lays out `document` like [`layout_document`], sizing replaced elements
/// with `images`.
///
/// Replaced elements are `<img>` and every `<svg>` element that is not inside
/// another SVG element. Each generates an atomic box of kind
/// [`crate::BoxKind::Replaced`] whose [`LayoutBox::image`] names the picture
/// to paint in its content box. Sizes follow CSS 2.2 §10.3.2, §10.4 and
/// §10.6.2: `width`/`height` (or the element's `width`/`height` attributes
/// when those are `auto`), the natural size and ratio from `images` (or, for
/// inline SVG, its `width`, `height`, and `viewBox` attributes), and
/// `min-*`/`max-*` sizes. A broken `<img>` with `alt` text and no specified
/// size is laid out as that text.
///
/// Limitations: `object-fit` and `object-position` are not supported (the
/// picture always stretches to the content box), `srcset`/`<picture>` are
/// ignored, and `min-*`/`max-*` sizes apply only to replaced elements.
pub fn layout_document_with_images(
    document: &HTMLDocument,
    styles: &css::ComputedStyles,
    viewport: Size,
    fonts: &dyn FontMetrics,
    images: &dyn ImageMetrics,
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
    let style_map = StyleMap::new(document, styles, root, viewport, fonts);
    let icb = build_initial_containing_block(document, &style_map, images);
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
            document.get_node(node).map(|node| node.get_kind()),
            Some(NodeKind::Element(element)) if element.get_name() == name
        )
    };
    let source = if background(root).is_some() {
        Some(root)
    } else if is_element(root, "html") {
        document
            .get_node(root)
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
