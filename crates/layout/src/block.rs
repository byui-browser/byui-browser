//! Block formatting: widths (CSS 2.2 §10.3), heights (§10.6), and margin
//! collapsing (§8.3.1).
//!
//! Every function returns fragments positioned relative to the border-box
//! origin of the box being laid out; the parent translates them into place.

use crate::box_tree::{BlockBox, BlockContents};
use crate::layout::Context;
use crate::style::{ComputedStyle, Dimension};
use crate::tree::{BoxKind, BoxStyle, LayoutBox};
use crate::{Edges, Rect};

/// Adjoining vertical margins, collapsed as CSS 2.2 §8.3.1 describes: the
/// largest positive margin plus the most negative margin.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct CollapsibleMargin {
    positive: f32,
    negative: f32,
}

impl CollapsibleMargin {
    pub(crate) fn new(margin: f32) -> Self {
        Self {
            positive: margin.max(0.0),
            negative: margin.min(0.0),
        }
    }

    pub(crate) fn adjoin(&mut self, other: Self) {
        self.positive = self.positive.max(other.positive);
        self.negative = self.negative.min(other.negative);
    }

    pub(crate) fn resolve(self) -> f32 {
        self.positive + self.negative
    }
}

/// The containing block a box is sized against (CSS 2.2 §10.1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContainingBlock {
    pub width: f32,
    /// `None` when the height is not definite, which makes percentage
    /// heights behave as `auto` (CSS 2.2 §10.5).
    pub height: Option<f32>,
}

/// A laid-out block-level box together with the margins its parent collapses.
pub(crate) struct BlockResult {
    /// The fragment with its border box at the origin.
    pub fragment: LayoutBox,
    /// The collapsed margin above the border box, including child margins
    /// that collapsed through the top edge.
    pub margin_top: CollapsibleMargin,
    /// The collapsed margin below the border box, including child margins
    /// that collapsed through the bottom edge.
    pub margin_bottom: CollapsibleMargin,
    /// Whether the box's top and bottom margins are adjoining (an empty box
    /// that margins collapse through).
    pub collapses_through: bool,
}

impl Context<'_> {
    /// Lays out a block-level, non-replaced box in normal flow. Widths follow
    /// CSS 2.2 §10.3.3 for left-to-right text.
    pub(crate) fn layout_block_level(&self, block: &BlockBox, cb: ContainingBlock) -> BlockResult {
        let style = &block.style;
        let basis = Some(cb.width);
        let padding = style
            .padding
            .map(|value| value.resolve_or_zero(basis))
            .horizontal();
        let margin_left = style.margin.left.resolve(basis);
        let margin_right = style.margin.right.resolve(basis);
        let (left, width, right) = match style.width.resolve(basis) {
            None => {
                // `auto` margins become 0 and the width fills the rest.
                let left = margin_left.unwrap_or(0.0);
                let right = margin_right.unwrap_or(0.0);
                let width = (cb.width - left - right - padding).max(0.0);
                log::trace!(
                    "node {:?}: auto width resolved to {width} in containing block {}",
                    block.node,
                    cb.width
                );
                (left, width, cb.width - left - padding - width)
            }
            Some(width) => {
                let fixed =
                    margin_left.unwrap_or(0.0) + margin_right.unwrap_or(0.0) + padding + width;
                let remaining = cb.width - padding - width;
                match (margin_left, margin_right) {
                    _ if fixed > cb.width => {
                        log::trace!(
                            "node {:?}: over-constrained width {width} in containing block {}; right margin absorbs the difference",
                            block.node,
                            cb.width
                        );
                        // Over-constrained with `auto` margins treated as 0:
                        // the right margin absorbs the difference.
                        let left = margin_left.unwrap_or(0.0);
                        (left, width, remaining - left)
                    }
                    (None, None) => {
                        log::trace!(
                            "node {:?}: auto left and right margins centered at {}",
                            block.node,
                            remaining / 2.0
                        );
                        (remaining / 2.0, width, remaining / 2.0)
                    }
                    (None, Some(right)) => (remaining - right, width, right),
                    (Some(left), _) => (left, width, remaining - left),
                }
            }
        };
        let kind = if block.node.is_some() {
            BoxKind::Block
        } else {
            BoxKind::AnonymousBlock
        };
        self.layout_block_container(block, cb, kind, width, (left, right))
    }

    /// Lays out an atomic inline-level block container (an inline-block):
    /// `auto` margins are zero and an `auto` width shrinks to fit
    /// (CSS 2.2 §10.3.9, §10.6.6).
    pub(crate) fn layout_inline_block(&self, block: &BlockBox, cb_width: f32) -> BlockResult {
        let style = &block.style;
        let basis = Some(cb_width);
        let padding = style
            .padding
            .map(|value| value.resolve_or_zero(basis))
            .horizontal();
        let margin_left = style.margin.left.resolve_or_zero(basis);
        let margin_right = style.margin.right.resolve_or_zero(basis);
        let width = style.width.resolve(basis).unwrap_or_else(|| {
            let (min_content, max_content) = self.intrinsic_content_sizes(block);
            let available = cb_width - margin_left - margin_right - padding;
            let width = max_content.min(available.max(min_content));
            log::trace!(
                "node {:?}: inline-block shrink-to-fit width {width} (min-content {min_content}, max-content {max_content}, available {available})",
                block.node
            );
            width
        });
        self.layout_block_container(
            block,
            ContainingBlock {
                width: cb_width,
                height: None,
            },
            BoxKind::InlineBlock,
            width,
            (margin_left, margin_right),
        )
    }

    /// Lays out the initial containing block: a viewport-sized block
    /// container that establishes the root block formatting context.
    pub(crate) fn layout_initial_containing_block(
        &self,
        block: &BlockBox,
        viewport: crate::Size,
    ) -> LayoutBox {
        let mut children = Vec::new();
        let mut lines = Vec::new();
        match &block.contents {
            BlockContents::Blocks(list) => {
                self.layout_flow(
                    list,
                    viewport.width,
                    Some(viewport.height),
                    FlowEdges::isolated(),
                    &mut children,
                );
            }
            BlockContents::Inlines(items) => {
                lines = self.layout_inline(items, &block.style, viewport.width);
            }
        }
        LayoutBox {
            kind: BoxKind::Viewport,
            node: None,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: viewport.width,
                height: viewport.height,
            },
            margin: Edges::default(),
            padding: Edges::default(),
            style: box_style(&block.style),
            children,
            lines,
        }
    }

    /// Lays out a block container once its used width and horizontal margins
    /// are known, computing its height (CSS 2.2 §10.6.3).
    fn layout_block_container(
        &self,
        block: &BlockBox,
        cb: ContainingBlock,
        kind: BoxKind,
        content_width: f32,
        (margin_left, margin_right): (f32, f32),
    ) -> BlockResult {
        let style = &block.style;
        let basis = Some(cb.width);
        let padding = style.padding.map(|value| value.resolve_or_zero(basis));
        let margin_top = style.margin.top.resolve_or_zero(basis);
        let margin_bottom = style.margin.bottom.resolve_or_zero(basis);
        // Percentage heights resolve only against a definite containing block
        // height; otherwise they behave as `auto` (CSS 2.2 §10.5).
        let specified_height = style.height.resolve(cb.height);

        let edges = FlowEdges {
            collapse_top: !block.establishes_bfc && padding.top == 0.0,
            collapse_bottom: !block.establishes_bfc
                && padding.bottom == 0.0
                && specified_height.is_none(),
            own_top: CollapsibleMargin::new(margin_top),
            own_bottom: CollapsibleMargin::new(margin_bottom),
        };
        let mut children = Vec::new();
        let mut lines = Vec::new();
        let flow = match &block.contents {
            BlockContents::Blocks(list) => {
                if matches!(
                    style.display,
                    crate::style::Display::Box {
                        inside: crate::style::DisplayInside::Flex,
                        ..
                    }
                ) {
                    self.layout_flex(list, content_width, specified_height, style, &mut children)
                } else {
                    self.layout_flow(list, content_width, specified_height, edges, &mut children)
                }
            }
            BlockContents::Inlines(items) => {
                lines = self.layout_inline(items, style, content_width);
                FlowResult {
                    height: lines.last().map_or(0.0, |line| line.rect.bottom()),
                    margin_top: edges.own_top,
                    margin_bottom: edges.own_bottom,
                    has_content: !lines.is_empty(),
                }
            }
        };
        let content_height = specified_height.unwrap_or(flow.height);
        if specified_height.is_none() {
            log::trace!(
                "node {:?}: auto height resolved to {content_height}",
                block.node
            );
        }
        let collapses_through = !block.establishes_bfc
            && !flow.has_content
            && padding.vertical() == 0.0
            && content_height == 0.0;

        for child in &mut children {
            child.translate(padding.left, padding.top);
        }
        for line in &mut lines {
            line.translate(padding.left, padding.top);
        }
        log::trace!(
            "laid out {kind:?} node {:?}: content {}x{}, margin top/bottom {margin_top}/{margin_bottom}, {} children, {} lines",
            block.node,
            content_width + padding.horizontal(),
            content_height + padding.vertical(),
            children.len(),
            lines.len()
        );
        BlockResult {
            fragment: LayoutBox {
                kind,
                node: block.node,
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: content_width + padding.horizontal(),
                    height: content_height + padding.vertical(),
                },
                margin: Edges {
                    top: margin_top,
                    right: margin_right,
                    bottom: margin_bottom,
                    left: margin_left,
                },
                padding,
                style: box_style(style),
                children,
                lines,
            },
            margin_top: flow.margin_top,
            margin_bottom: flow.margin_bottom,
            collapses_through,
        }
    }

    /// Stacks block-level children vertically, collapsing adjoining margins.
    /// Fragments are pushed relative to the parent's content-box origin.
    fn layout_flow(
        &self,
        list: &[BlockBox],
        content_width: f32,
        definite_height: Option<f32>,
        edges: FlowEdges,
        children: &mut Vec<LayoutBox>,
    ) -> FlowResult {
        let cb = ContainingBlock {
            width: content_width,
            height: definite_height,
        };
        let mut margin_top = edges.own_top;
        let mut pending = CollapsibleMargin::default();
        let mut cursor = 0.0_f32;
        let mut has_content = false;
        for child in list {
            let result = self.layout_block_level(child, cb);
            let y = if !has_content && edges.collapse_top {
                log::trace!("child margin collapses with parent top margin");
                // Still adjoining the parent's top margin: the child's margins
                // collapse with it and the child sits at the content edge.
                margin_top.adjoin(result.margin_top);
                if result.collapses_through {
                    log::trace!("empty child: margins collapse through it");
                    margin_top.adjoin(result.margin_bottom);
                } else {
                    has_content = true;
                    cursor = result.fragment.rect.height;
                    pending = result.margin_bottom;
                }
                0.0
            } else {
                pending.adjoin(result.margin_top);
                let y = cursor + pending.resolve();
                log::trace!(
                    "sibling margins collapsed to {} before child at y {y}",
                    pending.resolve()
                );
                if result.collapses_through {
                    pending.adjoin(result.margin_bottom);
                } else {
                    has_content = true;
                    cursor = y + result.fragment.rect.height;
                    pending = result.margin_bottom;
                }
                y
            };
            let mut fragment = result.fragment;
            fragment.translate(fragment.margin.left, y);
            children.push(fragment);
        }
        if edges.collapse_bottom {
            let mut margin_bottom = pending;
            margin_bottom.adjoin(edges.own_bottom);
            FlowResult {
                height: cursor,
                margin_top,
                margin_bottom,
                has_content,
            }
        } else {
            FlowResult {
                height: (cursor + pending.resolve()).max(0.0),
                margin_top,
                margin_bottom: edges.own_bottom,
                has_content,
            }
        }
    }

    /// Lays out a single flex line in row or column direction. Wrapping,
    /// shrink factors, alignment, and baseline alignment are not implemented.
    fn layout_flex(
        &self,
        list: &[BlockBox],
        width: f32,
        height: Option<f32>,
        style: &ComputedStyle,
        children: &mut Vec<LayoutBox>,
    ) -> FlowResult {
        if list.is_empty() {
            return FlowResult {
                height: 0.0,
                margin_top: CollapsibleMargin::default(),
                margin_bottom: CollapsibleMargin::default(),
                has_content: false,
            };
        }
        let column = style.flex_column;
        let available = if column { height.unwrap_or(0.0) } else { width };
        let gaps = style.gap * list.len().saturating_sub(1) as f32;
        let basis = list
            .iter()
            .map(|item| {
                if column {
                    item.style.height.resolve(height).unwrap_or_else(|| {
                        self.layout_block_level(
                            item,
                            ContainingBlock {
                                width,
                                height: None,
                            },
                        )
                        .fragment
                        .rect
                        .height
                    })
                } else {
                    item.style
                        .width
                        .resolve(Some(width))
                        .unwrap_or_else(|| self.intrinsic_content_sizes(item).1)
                }
            })
            .collect::<Vec<_>>();
        let grow: f32 = list.iter().map(|item| item.style.flex_grow).sum();
        let free = (available - gaps - basis.iter().sum::<f32>()).max(0.0);
        let mut cursor = 0.0;
        let mut cross = 0.0_f32;
        for (source, base) in list.iter().zip(basis) {
            let item = source;
            let share = if grow > 0.0 {
                free * item.style.flex_grow / grow
            } else {
                0.0
            };
            let target = base + share;
            let mut item = item.clone();
            let mut computed = (*item.style).clone();
            if column {
                computed.height = Dimension::Px(target);
            } else {
                computed.width = Dimension::Px(target);
            }
            item.style = std::rc::Rc::new(computed);
            let item_width = if column {
                width
            } else {
                target
                    + item
                        .style
                        .padding
                        .map(|value| value.resolve_or_zero(Some(width)))
                        .horizontal()
            };
            let result = self.layout_block_level(
                &item,
                ContainingBlock {
                    width: item_width,
                    height,
                },
            );
            let mut fragment = result.fragment;
            if column {
                fragment.translate(0.0, cursor + fragment.margin.top);
                cursor +=
                    fragment.rect.height + fragment.margin.top + fragment.margin.bottom + style.gap;
                cross = cross.max(fragment.rect.width);
            } else {
                fragment.translate(cursor + fragment.margin.left, 0.0);
                cursor +=
                    fragment.rect.width + fragment.margin.left + fragment.margin.right + style.gap;
                cross = cross.max(fragment.rect.height);
            }
            children.push(fragment);
        }
        FlowResult {
            height: if column { cursor - style.gap } else { cross },
            margin_top: CollapsibleMargin::default(),
            margin_bottom: CollapsibleMargin::default(),
            has_content: true,
        }
    }

    /// The min-content and max-content widths of a block container's content
    /// box (CSS Sizing 3 §5).
    pub(crate) fn intrinsic_content_sizes(&self, block: &BlockBox) -> (f32, f32) {
        match &block.contents {
            BlockContents::Blocks(list) => list
                .iter()
                .map(|child| self.intrinsic_outer_sizes(child))
                .fold((0.0_f32, 0.0_f32), |(min, max), (child_min, child_max)| {
                    (min.max(child_min), max.max(child_max))
                }),
            BlockContents::Inlines(items) => self.inline_intrinsic_sizes(items, &block.style),
        }
    }

    /// The min-content and max-content contributions of a box: its intrinsic
    /// content size, or its fixed `width`, plus horizontal margins and
    /// padding. Percentages and `auto` count as zero here.
    pub(crate) fn intrinsic_outer_sizes(&self, block: &BlockBox) -> (f32, f32) {
        let style = &block.style;
        let fixed = |value: Dimension| match value {
            Dimension::Px(px) => px,
            _ => 0.0,
        };
        let outer = fixed(style.padding.left)
            + fixed(style.padding.right)
            + fixed(style.margin.left)
            + fixed(style.margin.right);
        let (min, max) = match style.width {
            Dimension::Px(width) => (width, width),
            _ => self.intrinsic_content_sizes(block),
        };
        (min + outer, max + outer)
    }
}

/// How a block container's edges interact with its children's margins.
#[derive(Debug, Clone, Copy)]
struct FlowEdges {
    /// The first child's top margin adjoins the container's top margin.
    collapse_top: bool,
    /// The last child's bottom margin adjoins the container's bottom margin.
    collapse_bottom: bool,
    own_top: CollapsibleMargin,
    own_bottom: CollapsibleMargin,
}

impl FlowEdges {
    /// Edges of a formatting context root with no margins of its own.
    fn isolated() -> Self {
        Self {
            collapse_top: false,
            collapse_bottom: false,
            own_top: CollapsibleMargin::default(),
            own_bottom: CollapsibleMargin::default(),
        }
    }
}

struct FlowResult {
    /// Height of the content from its top edge to the last child's bottom
    /// border edge, plus that child's margin when it does not collapse
    /// through the bottom.
    height: f32,
    margin_top: CollapsibleMargin,
    margin_bottom: CollapsibleMargin,
    /// Whether any in-flow child or line box keeps margins from collapsing
    /// through the container.
    has_content: bool,
}

pub(crate) fn box_style(style: &ComputedStyle) -> BoxStyle {
    BoxStyle {
        background_color: style.background_color,
        text_color: style.color,
        border_radius: style.border_radius,
    }
}
