//! Floats (CSS 2.2 §9.5) and positioning schemes (CSS 2.2 §9.3, §9.4.3,
//! §10.3.7, §10.6.4).
//!
//! Floats are placed in a [`FloatContext`] owned by the block formatting
//! context root they belong to. Descendant block containers reach it through
//! a [`FloatScope`] that records their offset from the root, so line boxes
//! anywhere in the formatting context shorten beside floats.
//!
//! Absolutely positioned boxes are collected as [`PendingAbsolute`] entries
//! with their static position and handed up the fragment tree until they
//! reach their containing block: the nearest positioned ancestor block box
//! (or the initial containing block). `position: fixed` boxes always reach
//! the initial containing block.
//!
//! Limitations: `clear` is not supported; in-flow block formatting context
//! roots (such as `display: flow-root`) do not avoid floats; a floated box's
//! y position is estimated before its parent's margins finish collapsing;
//! relatively positioned inline boxes move with their text but do not become
//! containing blocks; and `position: sticky` behaves as `relative` with no
//! offset.

use crate::Rect;
use crate::block::{BlockResult, ContainingBlock};
use crate::box_tree::BlockBox;
use crate::layout::Context;
use crate::style::{BoxSizing, ComputedStyle, Dimension, Float, Position};
use crate::tree::{BoxKind, LayoutBox};

/// Tolerance for floating-point error when testing whether a float fits.
const EPSILON: f32 = 0.001;

/// One float's margin box in its formatting context's coordinates.
#[derive(Debug, Clone, Copy)]
struct PlacedFloat {
    left: bool,
    rect: Rect,
}

/// The floats placed so far in one block formatting context.
#[derive(Debug, Default)]
pub(crate) struct FloatContext {
    floats: Vec<PlacedFloat>,
}

impl FloatContext {
    /// Floats whose margin box vertically overlaps `[y, y + height)` and
    /// horizontally overlaps `[x_min, x_max)`. Empty floats take no room.
    fn overlapping(
        &self,
        y: f32,
        height: f32,
        x_min: f32,
        x_max: f32,
    ) -> impl Iterator<Item = &PlacedFloat> {
        let bottom = y + height.max(EPSILON);
        self.floats.iter().filter(move |float| {
            float.rect.width > 0.0
                && float.rect.height > 0.0
                && float.rect.y < bottom
                && float.rect.bottom() > y
                && float.rect.x < x_max
                && float.rect.right() > x_min
        })
    }

    /// The free horizontal space `(left, right)` between floats across the
    /// band `[y, y + height)`, clamped to `[x_min, x_max]`.
    fn band(&self, y: f32, height: f32, x_min: f32, x_max: f32) -> (f32, f32) {
        self.overlapping(y, height, x_min, x_max)
            .fold((x_min, x_max), |(left, right), float| {
                if float.left {
                    (left.max(float.rect.right()), right)
                } else {
                    (left, right.min(float.rect.x))
                }
            })
    }

    /// The nearest float bottom below `y` among floats overlapping the band,
    /// where the free space may next change.
    fn next_edge(&self, y: f32, height: f32, x_min: f32, x_max: f32) -> Option<f32> {
        self.overlapping(y, height, x_min, x_max)
            .map(|float| float.rect.bottom())
            .filter(|bottom| *bottom > y)
            .min_by(f32::total_cmp)
    }

    /// The lowest float margin edge, or `None` without floats.
    fn bottom(&self) -> Option<f32> {
        self.floats
            .iter()
            .map(|float| float.rect.bottom())
            .max_by(f32::total_cmp)
    }

    /// Places a float's `width` × `height` margin box as high as possible at
    /// or below `min_y`, then as far left (or right) as possible, following
    /// CSS 2.2 §9.5.1: it is no higher than an earlier float and moves down
    /// past floats until it fits. A float wider than the containing block is
    /// placed once no other float is beside it.
    fn place(
        &mut self,
        left: bool,
        width: f32,
        height: f32,
        min_y: f32,
        x_min: f32,
        x_max: f32,
    ) -> (f32, f32) {
        let earlier_top = self
            .floats
            .iter()
            .map(|float| float.rect.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let mut y = min_y.max(earlier_top);
        let (start, end) = loop {
            let (start, end) = self.band(y, height, x_min, x_max);
            if end - start + EPSILON >= width {
                break (start, end);
            }
            match self.next_edge(y, height, x_min, x_max) {
                Some(next) => y = next,
                None => break (start, end),
            }
        };
        let x = if left { start } else { end - width };
        self.floats.push(PlacedFloat {
            left,
            rect: Rect {
                x,
                y,
                width,
                height,
            },
        });
        log::trace!(
            "float {} placed at ({x}, {y}), {width}x{height}",
            if left { "left" } else { "right" }
        );
        (x, y)
    }
}

/// A block container's view of its formatting context's floats: the context
/// and the container's content-box origin within it. All positions taken and
/// returned are relative to that origin.
pub(crate) struct FloatScope<'f> {
    context: &'f mut FloatContext,
    x: f32,
    y: f32,
}

impl<'f> FloatScope<'f> {
    /// The scope of a formatting context root at the context's origin.
    pub(crate) fn new(context: &'f mut FloatContext) -> Self {
        Self {
            context,
            x: 0.0,
            y: 0.0,
        }
    }

    /// This scope with its origin moved by `dx`, `dy`.
    pub(crate) fn offset(self, dx: f32, dy: f32) -> Self {
        Self {
            context: self.context,
            x: self.x + dx,
            y: self.y + dy,
        }
    }

    /// A shorter-lived scope sharing the same floats, with its origin moved
    /// by `dx`, `dy`.
    pub(crate) fn reborrow(&mut self, dx: f32, dy: f32) -> FloatScope<'_> {
        FloatScope {
            context: self.context,
            x: self.x + dx,
            y: self.y + dy,
        }
    }

    /// The `(x, width)` a line box `height` tall at `y` may use within a
    /// container `width` wide.
    pub(crate) fn line_band(&self, y: f32, height: f32, width: f32) -> (f32, f32) {
        let (left, right) = self
            .context
            .band(self.y + y, height, self.x, self.x + width);
        (left - self.x, (right - left).max(0.0))
    }

    /// Whether any float narrows a line `height` tall at `y`, and if so the
    /// next `y` below it where the space may widen.
    pub(crate) fn next_line_edge(&self, y: f32, height: f32, width: f32) -> Option<f32> {
        self.context
            .next_edge(self.y + y, height, self.x, self.x + width)
            .map(|edge| edge - self.y)
    }

    /// The lowest float margin edge relative to this scope, if any.
    pub(crate) fn bottom(&self) -> Option<f32> {
        self.context.bottom().map(|bottom| bottom - self.y)
    }

    fn place(
        &mut self,
        left: bool,
        size: (f32, f32),
        min_y: f32,
        container_width: f32,
    ) -> (f32, f32) {
        let (x, y) = self.context.place(
            left,
            size.0,
            size.1,
            self.y + min_y,
            self.x,
            self.x + container_width,
        );
        (x - self.x, y - self.y)
    }
}

/// An absolutely positioned box waiting for its containing block to be laid
/// out.
#[derive(Debug, Clone)]
pub(crate) struct PendingAbsolute {
    pub block: BlockBox,
    /// The static position (CSS 2.2 §10.3.7, §10.6.4): where the box's top
    /// left margin edge would have been in flow, relative to the origin of
    /// the fragment currently holding this entry.
    pub x: f32,
    pub y: f32,
}

impl PendingAbsolute {
    pub(crate) fn translate(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }
}

impl BlockResult {
    /// Moves the fragment's border box to `(x, y)` plus its relative
    /// offset, carrying its pending absolutely positioned descendants along.
    pub(crate) fn place(self, x: f32, y: f32) -> (LayoutBox, Vec<PendingAbsolute>) {
        let (dx, dy) = (x + self.relative.0, y + self.relative.1);
        let mut fragment = self.fragment;
        fragment.translate(dx, dy);
        let mut absolutes = self.absolutes;
        for absolute in &mut absolutes {
            absolute.translate(dx, dy);
        }
        (fragment, absolutes)
    }
}

/// The offset of a relatively positioned box (CSS 2.2 §9.4.3). `left` wins
/// over `right` and `top` over `bottom`; percentages of an indefinite height
/// count as `auto`.
pub(crate) fn relative_offset(style: &ComputedStyle, cb: ContainingBlock) -> (f32, f32) {
    if style.position != Position::Relative {
        return (0.0, 0.0);
    }
    let axis = |start: Dimension, end: Dimension, basis: Option<f32>| match (
        start.resolve(basis),
        end.resolve(basis),
    ) {
        (Some(start), _) => start,
        (None, Some(end)) => -end,
        (None, None) => 0.0,
    };
    (
        axis(style.inset.left, style.inset.right, Some(cb.width)),
        axis(style.inset.top, style.inset.bottom, cb.height),
    )
}

impl Context<'_> {
    /// Lays out a float with a shrink-to-fit width (CSS 2.2 §10.3.5) and
    /// places it in `floats` no higher than `min_y`. Returns the fragment
    /// relative to the scope's origin.
    pub(crate) fn layout_float(
        &self,
        block: &BlockBox,
        container_width: f32,
        floats: &mut FloatScope<'_>,
        min_y: f32,
    ) -> (LayoutBox, Vec<PendingAbsolute>) {
        let kind = if block.node.is_some() {
            BoxKind::Block
        } else {
            BoxKind::AnonymousBlock
        };
        let result = self.layout_shrink_to_fit(block, container_width, kind);
        let fragment = &result.fragment;
        let margin = fragment.margin;
        let size = (
            margin.horizontal() + fragment.rect.width,
            margin.vertical() + fragment.rect.height,
        );
        let (x, y) = floats.place(
            block.style.float == Float::Left,
            size,
            min_y,
            container_width,
        );
        result.place(x + margin.left, y + margin.top)
    }

    /// Lays out every pending box that `cb` (a padding box in the same
    /// coordinates as the entries) contains, pushing the fragments onto
    /// `children`. Fixed boxes are kept unless `is_viewport`, and returned
    /// for an ancestor to place.
    pub(crate) fn place_absolutes(
        &self,
        pending: Vec<PendingAbsolute>,
        cb: Rect,
        is_viewport: bool,
        children: &mut Vec<LayoutBox>,
    ) -> Vec<PendingAbsolute> {
        let mut queue = std::collections::VecDeque::from(pending);
        let mut remaining = Vec::new();
        while let Some(entry) = queue.pop_front() {
            if entry.block.style.position == Position::Fixed && !is_viewport {
                remaining.push(entry);
                continue;
            }
            let (fragment, nested) = self.layout_absolute(&entry, cb);
            children.push(fragment);
            // The box contains its absolute descendants itself; only fixed
            // ones come back, and they belong to the viewport.
            if is_viewport {
                queue.extend(nested);
            } else {
                remaining.extend(nested);
            }
        }
        remaining
    }

    /// Lays out an absolutely positioned box against its containing block's
    /// padding box `cb` (CSS 2.2 §10.3.7, §10.6.4). The result is in `cb`'s
    /// coordinates.
    ///
    /// `auto` insets fall back to the static position; with both insets on an
    /// axis set, `auto` sizes stretch between them and `auto` margins center
    /// the box. Otherwise an `auto` width shrinks to fit. Min/max sizes are
    /// not applied.
    fn layout_absolute(
        &self,
        pending: &PendingAbsolute,
        cb: Rect,
    ) -> (LayoutBox, Vec<PendingAbsolute>) {
        let block = &pending.block;
        let style = &block.style;
        let basis = Some(cb.width);
        let padding = style.padding.map(|value| value.resolve_or_zero(basis));
        let inset_left = style.inset.left.resolve(Some(cb.width));
        let inset_right = style.inset.right.resolve(Some(cb.width));
        let inset_top = style.inset.top.resolve(Some(cb.height));
        let inset_bottom = style.inset.bottom.resolve(Some(cb.height));
        let replaced = match &block.contents {
            crate::box_tree::BlockContents::Replaced(replaced) => {
                Some(replaced.used_size(style, basis, Some(cb.height)))
            }
            _ => None,
        };

        // Horizontal: CSS 2.2 §10.3.7.
        let margin_left = style.margin.left.resolve(basis);
        let margin_right = style.margin.right.resolve(basis);
        let static_left = pending.x - cb.x;
        let specified_width = replaced
            .map(|(width, _)| width)
            .or(style.content_width(basis));
        let (left, width, margin_left, margin_right) = match (inset_left, inset_right) {
            (Some(left), Some(right)) => {
                let (fixed_left, fixed_right) =
                    (margin_left.unwrap_or(0.0), margin_right.unwrap_or(0.0));
                match specified_width {
                    None => {
                        let width = (cb.width
                            - left
                            - right
                            - fixed_left
                            - fixed_right
                            - padding.horizontal())
                        .max(0.0);
                        (left, width, fixed_left, fixed_right)
                    }
                    Some(width) => {
                        let free = cb.width - left - right - width - padding.horizontal();
                        let (margin_left, margin_right) = match (margin_left, margin_right) {
                            (None, None) if free >= 0.0 => (free / 2.0, free / 2.0),
                            (None, None) => (0.0, free),
                            (None, Some(right)) => (free - right, right),
                            (Some(left), _) => (left, free - left),
                        };
                        (left, width, margin_left, margin_right)
                    }
                }
            }
            (left, right) => {
                let (margin_left, margin_right) =
                    (margin_left.unwrap_or(0.0), margin_right.unwrap_or(0.0));
                let start = left.unwrap_or(if right.is_some() { 0.0 } else { static_left });
                let width = specified_width.unwrap_or_else(|| {
                    let (min_content, max_content) = self.intrinsic_content_sizes(block);
                    let available = cb.width
                        - start
                        - right.unwrap_or(0.0)
                        - margin_left
                        - margin_right
                        - padding.horizontal();
                    max_content.min(available.max(min_content))
                });
                let left = match (left, right) {
                    (Some(left), _) => left,
                    (None, Some(right)) => {
                        cb.width - right - margin_right - width - padding.horizontal() - margin_left
                    }
                    (None, None) => static_left,
                };
                (left, width, margin_left, margin_right)
            }
        };

        // Vertical: CSS 2.2 §10.6.4. An `auto` height between two insets
        // is fixed before layout; otherwise the content decides it.
        let margin_top = style.margin.top.resolve(basis);
        let margin_bottom = style.margin.bottom.resolve(basis);
        let specified_height = replaced
            .map(|(_, height)| height)
            .or(style.content_height(basis, Some(cb.height)));
        let mut laid_out = block.clone();
        if let (None, Some(top), Some(bottom)) = (specified_height, inset_top, inset_bottom) {
            let height = (cb.height
                - top
                - bottom
                - margin_top.unwrap_or(0.0)
                - margin_bottom.unwrap_or(0.0)
                - padding.vertical())
            .max(0.0);
            let mut computed = (**style).clone();
            // `height` is a content size, so lay the box out as `content-box`.
            computed.height = Dimension::Px(height);
            computed.box_sizing = BoxSizing::ContentBox;
            laid_out.style = std::rc::Rc::new(computed);
        }
        let result = self.layout_block_container_at(
            &laid_out,
            ContainingBlock {
                width: cb.width,
                height: Some(cb.height),
            },
            width,
            (margin_left, margin_right),
        );
        let height = result.fragment.rect.height;
        let (margin_top, margin_bottom) = match (inset_top, inset_bottom, specified_height) {
            (Some(top), Some(bottom), Some(_)) => {
                let free = cb.height - top - bottom - height;
                match (margin_top, margin_bottom) {
                    (None, None) => (free / 2.0, free / 2.0),
                    (None, Some(bottom)) => (free - bottom, bottom),
                    (Some(top), bottom) => (top, bottom.unwrap_or(free - top)),
                }
            }
            _ => (margin_top.unwrap_or(0.0), margin_bottom.unwrap_or(0.0)),
        };
        let top = match (inset_top, inset_bottom) {
            (Some(top), _) => top,
            (None, Some(bottom)) => cb.height - bottom - margin_bottom - height - margin_top,
            (None, None) => pending.y - cb.y,
        };
        log::trace!(
            "absolute box {:?} placed at ({left}, {top}) in containing block {cb:?}",
            block.node
        );
        let (mut fragment, nested) =
            result.place(cb.x + left + margin_left, cb.y + top + margin_top);
        fragment.margin.top = margin_top;
        fragment.margin.bottom = margin_bottom;
        (fragment, nested)
    }
}
