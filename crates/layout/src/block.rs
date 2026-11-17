//! Block formatting: widths (CSS 2.2 §10.3), heights (§10.6), and margin
//! collapsing (§8.3.1).
//!
//! Every function returns fragments positioned relative to the border-box
//! origin of the box being laid out; the parent translates them into place.

use crate::box_tree::{BlockBox, BlockContents};
use crate::layout::Context;
use std::sync::Arc;

use crate::positioned::{FloatContext, FloatScope, PendingAbsolute, relative_offset};
use crate::style::{BoxSizing, ComputedStyle, Dimension, Position};
use crate::tree::{BoxKind, BoxStyle, LayoutBox, LineBox, LineFragment, Placement};
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
    /// The `position: relative` offset to add once the box is placed.
    pub relative: (f32, f32),
    /// Absolutely positioned descendants this box does not contain, relative
    /// to its border-box origin.
    pub absolutes: Vec<PendingAbsolute>,
}

impl Context<'_> {
    /// Lays out a block-level, non-replaced box in normal flow. Widths follow
    /// CSS 2.2 §10.3.3 for left-to-right text.
    ///
    /// `floats` is the parent's float scope moved to where the box's top
    /// margin edge is expected; `None` lays the box out without floats.
    pub(crate) fn layout_block_level(
        &self,
        block: &BlockBox,
        cb: ContainingBlock,
        floats: Option<FloatScope<'_>>,
    ) -> BlockResult {
        let style = &block.style;
        let basis = Some(cb.width);
        let padding = style.outer_edges(basis).horizontal();
        let margin_left = style.margin.left.resolve(basis);
        let margin_right = style.margin.right.resolve(basis);
        let used_width = match &block.contents {
            // A replaced box's width is never `auto` by now (CSS 2.2
            // §10.3.4): it comes from its natural size or ratio.
            BlockContents::Replaced(replaced) => {
                Some(replaced.used_size(style, basis, cb.height).0)
            }
            _ => style
                .content_width(basis)
                .or(self.control_size(block, basis, cb.height).width),
        };
        let (left, width, right) = match used_width {
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
        let floats = floats.map(|floats| floats.offset(left, 0.0));
        self.layout_block_container(block, cb, kind, width, (left, right), floats)
    }

    /// Lays out an atomic inline-level block container (an inline-block):
    /// `auto` margins are zero and an `auto` width shrinks to fit
    /// (CSS 2.2 §10.3.9, §10.6.6).
    pub(crate) fn layout_inline_block(&self, block: &BlockBox, cb_width: f32) -> BlockResult {
        self.layout_shrink_to_fit(block, cb_width, BoxKind::InlineBlock)
    }

    /// Lays out a box whose `auto` margins are zero and whose `auto` width
    /// shrinks to fit, as inline-blocks and floats are (CSS 2.2 §10.3.5,
    /// §10.3.9).
    pub(crate) fn layout_shrink_to_fit(
        &self,
        block: &BlockBox,
        cb_width: f32,
        kind: BoxKind,
    ) -> BlockResult {
        let style = &block.style;
        let basis = Some(cb_width);
        let padding = style.outer_edges(basis).horizontal();
        let margin_left = style.margin.left.resolve_or_zero(basis);
        let margin_right = style.margin.right.resolve_or_zero(basis);
        let replaced_width = match &block.contents {
            BlockContents::Replaced(replaced) => Some(replaced.used_size(style, basis, None).0),
            _ => None,
        };
        let width = replaced_width
            .or(style.content_width(basis))
            .or(self.control_size(block, basis, None).width)
            .unwrap_or_else(|| {
            let (min_content, max_content) = self.intrinsic_content_sizes(block);
            let available = cb_width - margin_left - margin_right - padding;
            let width = max_content.min(available.max(min_content));
            log::trace!(
                "node {:?}: shrink-to-fit width {width} (min-content {min_content}, max-content {max_content}, available {available})",
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
            kind,
            width,
            (margin_left, margin_right),
            None,
        )
    }

    /// Lays out a block container whose used content width and horizontal
    /// margins are already known, as for absolutely positioned boxes.
    pub(crate) fn layout_block_container_at(
        &self,
        block: &BlockBox,
        cb: ContainingBlock,
        content_width: f32,
        margins: (f32, f32),
    ) -> BlockResult {
        let kind = if block.node.is_some() {
            BoxKind::Block
        } else {
            BoxKind::AnonymousBlock
        };
        self.layout_block_container(block, cb, kind, content_width, margins, None)
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
        let mut float_context = FloatContext::default();
        let mut floats = FloatScope::new(&mut float_context);
        let absolutes = match &block.contents {
            BlockContents::Blocks(list) => {
                self.layout_flow(
                    list,
                    viewport.width,
                    Some(viewport.height),
                    FlowEdges::isolated(),
                    &mut children,
                    &mut floats,
                )
                .absolutes
            }
            BlockContents::Inlines(items) => {
                let inline = self.layout_inline(items, &block.style, viewport.width, &mut floats);
                lines = inline.lines;
                children.extend(inline.floats);
                inline.absolutes
            }
            // The initial containing block is never generated by an element.
            BlockContents::Replaced(_) => Vec::new(),
        };
        let viewport_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: viewport.width,
            height: viewport.height,
        };
        self.place_absolutes(absolutes, viewport_rect, true, &mut children);
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
            placement: Placement::InFlow,
            children,
            lines,
            image: None,
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
        floats: Option<FloatScope<'_>>,
    ) -> BlockResult {
        let kind = match block.contents {
            BlockContents::Replaced(_) => BoxKind::Replaced,
            _ => kind,
        };
        let style = &block.style;
        let basis = Some(cb.width);
        let padding = style.outer_edges(basis);
        let margin_top = style.margin.top.resolve_or_zero(basis);
        let margin_bottom = style.margin.bottom.resolve_or_zero(basis);
        // Percentage heights resolve only against a definite containing block
        // height; otherwise they behave as `auto` (CSS 2.2 §10.5).
        let control = self.control_size(block, basis, cb.height);
        let specified_height = match &block.contents {
            BlockContents::Replaced(replaced) => {
                Some(replaced.used_size(style, basis, cb.height).1)
            }
            _ => style.content_height(basis, cb.height).or(control.height),
        };

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
        // A formatting context root keeps its floats to itself; any other
        // block container shares its parent's, from its content-box origin.
        let mut own_floats = FloatContext::default();
        let scope = match floats {
            Some(floats) if !block.establishes_bfc => floats,
            _ => FloatScope::new(&mut own_floats),
        };
        let mut floats = scope.offset(padding.left, padding.top);
        let mut flow = match &block.contents {
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
                    self.layout_flow(
                        list,
                        content_width,
                        specified_height,
                        edges,
                        &mut children,
                        &mut floats,
                    )
                }
            }
            BlockContents::Inlines(items) => {
                let inline = self.layout_inline(items, style, content_width, &mut floats);
                lines = inline.lines;
                children.extend(inline.floats);
                let height = match lines.last() {
                    Some(line) => line.rect.bottom(),
                    // A single-line control with no text (an empty field or
                    // `<input type=button>`) keeps one strut-high line.
                    None if block.centers_lines => {
                        let metrics = self
                            .fonts
                            .font_vertical_metrics(style.font, style.font_size);
                        metrics.ascent + metrics.descent + metrics.line_gap
                    }
                    None => 0.0,
                };
                FlowResult {
                    height,
                    margin_top: edges.own_top,
                    margin_bottom: edges.own_bottom,
                    has_content: !lines.is_empty(),
                    absolutes: inline.absolutes,
                }
            }
            BlockContents::Replaced(_) => FlowResult {
                height: 0.0,
                margin_top: edges.own_top,
                margin_bottom: edges.own_bottom,
                has_content: true,
                absolutes: Vec::new(),
            },
        };
        // A formatting context root's `auto` height grows to contain its
        // floats (CSS 2.2 §10.6.7).
        if block.establishes_bfc
            && let Some(bottom) = floats.bottom()
        {
            flow.height = flow.height.max(bottom);
        }
        let content_height = specified_height.unwrap_or(flow.height.max(control.min_height));
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
        // Single-line controls center their text vertically (CSS Basic UI
        // leaves this to the user agent; Chrome centers it). In a box shorter
        // than its line the text overflows both edges equally and is clipped.
        let line_offset = if block.centers_lines {
            (content_height - flow.height) / 2.0
        } else {
            0.0
        };
        for line in &mut lines {
            line.translate(padding.left, padding.top + line_offset);
        }
        if block.clips_contents {
            // Control text is clipped to the padding box, inside the borders.
            let border = style.borders.border_width;
            let clip = Rect {
                x: border.left,
                y: border.top,
                width: content_width + padding.horizontal() - border.horizontal(),
                height: content_height + padding.vertical() - border.vertical(),
            };
            clip_text(&mut lines, clip);
        }
        let mut absolutes = flow.absolutes;
        for absolute in &mut absolutes {
            absolute.translate(padding.left, padding.top);
        }
        let border_box = Rect {
            x: 0.0,
            y: 0.0,
            width: content_width + padding.horizontal(),
            height: content_height + padding.vertical(),
        };
        // A positioned box is the containing block of its absolutely
        // positioned descendants: its padding box (CSS 2.2 §10.1).
        if style.is_positioned() && !absolutes.is_empty() {
            let b = style.borders.border_width;
            let padding_box = Rect {
                x: b.left,
                y: b.top,
                width: (border_box.width - b.horizontal()).max(0.0),
                height: (border_box.height - b.vertical()).max(0.0),
            };
            absolutes = self.place_absolutes(absolutes, padding_box, false, &mut children);
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
                rect: border_box,
                margin: Edges {
                    top: margin_top,
                    right: margin_right,
                    bottom: margin_bottom,
                    left: margin_left,
                },
                padding: style.padding.map(|v| v.resolve_or_zero(basis)),
                style: box_style(style),
                placement: placement(style),
                children,
                lines,
                image: match &block.contents {
                    BlockContents::Replaced(replaced) => Some(replaced.key.clone()),
                    _ => None,
                },
            },
            margin_top: flow.margin_top,
            margin_bottom: flow.margin_bottom,
            collapses_through,
            relative: relative_offset(style, cb),
            absolutes,
        }
    }

    /// Stacks block-level children vertically, collapsing adjoining margins.
    /// Fragments are pushed relative to the parent's content-box origin.
    ///
    /// Floats are placed in `floats` and absolutely positioned children are
    /// returned as pending, both at the position the next in-flow box would
    /// take.
    fn layout_flow(
        &self,
        list: &[BlockBox],
        content_width: f32,
        definite_height: Option<f32>,
        edges: FlowEdges,
        children: &mut Vec<LayoutBox>,
        floats: &mut FloatScope<'_>,
    ) -> FlowResult {
        let cb = ContainingBlock {
            width: content_width,
            height: definite_height,
        };
        let mut margin_top = edges.own_top;
        let mut pending = CollapsibleMargin::default();
        let mut cursor = 0.0_f32;
        let mut has_content = false;
        let mut absolutes = Vec::new();
        for child in list {
            let still_collapsing_top = !has_content && edges.collapse_top;
            if child.style.is_out_of_flow() {
                let y = if still_collapsing_top {
                    0.0
                } else {
                    cursor + pending.resolve()
                };
                if child.style.is_floated() {
                    let (fragment, nested) = self.layout_float(child, content_width, floats, y);
                    children.push(fragment);
                    absolutes.extend(nested);
                } else {
                    absolutes.push(PendingAbsolute {
                        block: child.clone(),
                        x: 0.0,
                        y,
                    });
                }
                continue;
            }
            // Where the child's top border edge is expected, so floats can
            // be found before its own margins finish collapsing.
            let expected_y = if still_collapsing_top {
                0.0
            } else {
                let mut margin = pending;
                margin.adjoin(CollapsibleMargin::new(
                    child.style.margin.top.resolve_or_zero(Some(content_width)),
                ));
                cursor + margin.resolve()
            };
            let result = self.layout_block_level(child, cb, Some(floats.reborrow(0.0, expected_y)));
            let y = if still_collapsing_top {
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
            let margin_left = result.fragment.margin.left;
            let (fragment, nested) = result.place(margin_left, y);
            children.push(fragment);
            absolutes.extend(nested);
        }
        if edges.collapse_bottom {
            let mut margin_bottom = pending;
            margin_bottom.adjoin(edges.own_bottom);
            FlowResult {
                height: cursor,
                margin_top,
                margin_bottom,
                has_content,
                absolutes,
            }
        } else {
            FlowResult {
                height: (cursor + pending.resolve()).max(0.0),
                margin_top,
                margin_bottom: edges.own_bottom,
                has_content,
                absolutes,
            }
        }
    }

    /// Lays out a single flex line in row or column direction. Wrapping,
    /// shrink factors, alignment, and baseline alignment are not implemented.
    ///
    /// Absolutely positioned children are not flex items; their static
    /// position is the container's content-box origin. `float` does not apply
    /// to flex items, so floated children are laid out as ordinary items.
    fn layout_flex(
        &self,
        list: &[BlockBox],
        width: f32,
        height: Option<f32>,
        style: &ComputedStyle,
        children: &mut Vec<LayoutBox>,
    ) -> FlowResult {
        let (absolute, items): (Vec<&BlockBox>, Vec<&BlockBox>) = list
            .iter()
            .partition(|item| item.style.is_absolutely_positioned());
        let mut absolutes = absolute
            .into_iter()
            .map(|block| PendingAbsolute {
                block: block.clone(),
                x: 0.0,
                y: 0.0,
            })
            .collect::<Vec<_>>();
        let list = items;
        if list.is_empty() {
            return FlowResult {
                height: 0.0,
                margin_top: CollapsibleMargin::default(),
                margin_bottom: CollapsibleMargin::default(),
                has_content: false,
                absolutes,
            };
        }
        let column = style.flex_column;
        let available = if column { height.unwrap_or(0.0) } else { width };
        let gaps = style.gap * list.len().saturating_sub(1) as f32;
        // Each item's main-axis padding: free space is what is left after
        // the items' outer sizes, not just their content sizes.
        let main_padding = |item: &BlockBox| {
            let padding = item.style.outer_edges(Some(width));
            if column {
                padding.vertical()
            } else {
                padding.horizontal()
            }
        };
        // The border-box height of an item laid out at its natural height,
        // converted to a content-box height.
        let natural_content_height = |item: &BlockBox| {
            let border_box = self
                .layout_block_level(
                    item,
                    ContainingBlock {
                        width,
                        height: None,
                    },
                    None,
                )
                .fragment
                .rect
                .height;
            (border_box - main_padding(item)).max(0.0)
        };
        // Content-box sizes along the main axis. `flex-basis`, like the main
        // size property, measures the box `box-sizing` selects.
        let basis = list
            .iter()
            .map(|item| {
                let basis_size = if column {
                    item.style
                        .content_height_for(item.style.flex_basis, Some(width), height)
                } else {
                    item.style
                        .content_width_for(item.style.flex_basis, Some(width))
                };
                basis_size.unwrap_or_else(|| {
                    if column {
                        item.style
                            .content_height(Some(width), height)
                            .unwrap_or_else(|| natural_content_height(item))
                    } else {
                        item.style
                            .content_width(Some(width))
                            .unwrap_or_else(|| self.intrinsic_content_sizes(item).1)
                    }
                })
            })
            .collect::<Vec<_>>();
        // Content-box minimum main sizes that shrinking never goes below.
        let minimums = list
            .iter()
            .map(|item| {
                if column {
                    if item.style.min_height_auto {
                        natural_content_height(item)
                    } else {
                        item.style
                            .content_height_for(item.style.min_height, Some(width), height)
                            .unwrap_or(0.0)
                    }
                } else if item.style.min_width_auto {
                    self.intrinsic_content_sizes(item).0
                } else {
                    item.style
                        .content_width_for(item.style.min_width, Some(width))
                        .unwrap_or(0.0)
                }
                .max(0.0)
            })
            .collect::<Vec<_>>();
        let outer: f32 = list
            .iter()
            .zip(&basis)
            .map(|(item, base)| base + main_padding(item))
            .sum();
        let grow: f32 = list.iter().map(|item| item.style.flex_grow).sum();
        let free = available - gaps - outer;
        let shrink: f32 = list
            .iter()
            .zip(&basis)
            .map(|(item, base)| item.style.flex_shrink * base.max(0.0))
            .sum();
        let mut cursor = 0.0;
        let mut cross = 0.0_f32;
        for ((source, base), minimum) in list.iter().zip(basis).zip(minimums) {
            let item = source;
            let target = if free >= 0.0 && grow > 0.0 {
                base + free * item.style.flex_grow / grow
            } else if free < 0.0 && shrink > 0.0 {
                (base + free * item.style.flex_shrink * base.max(0.0) / shrink).max(minimum)
            } else {
                base
            };
            let mut item = (*item).clone();
            let mut computed = (*item.style).clone();
            // `target` is a content size, whatever the item's `box-sizing`.
            // Convert the cross size to a content size too before switching
            // the item to `content-box`, so a border-box cross size keeps
            // its padding inside it.
            if computed.box_sizing == BoxSizing::BorderBox {
                if column {
                    if let Some(content) = computed.content_width(Some(width)) {
                        computed.width = Dimension::Px(content);
                    }
                } else if let Some(content) = computed.content_height(Some(width), height) {
                    computed.height = Dimension::Px(content);
                }
            }
            computed.box_sizing = BoxSizing::ContentBox;
            if column {
                computed.height = Dimension::Px(target);
            } else {
                computed.width = Dimension::Px(target);
            }
            item.style = std::rc::Rc::new(computed);
            let item_width = if column {
                width
            } else {
                target + item.style.outer_edges(Some(width)).horizontal()
            };
            let result = self.layout_block_level(
                &item,
                ContainingBlock {
                    width: item_width,
                    height,
                },
                None,
            );
            let (margin, size) = (result.fragment.margin, result.fragment.rect);
            let (mut fragment, nested) = if column {
                let placed = result.place(0.0, cursor + margin.top);
                cursor += size.height + margin.vertical() + style.gap;
                cross = cross.max(size.width);
                placed
            } else {
                let placed = result.place(cursor + margin.left, 0.0);
                cursor += size.width + margin.horizontal() + style.gap;
                cross = cross.max(size.height);
                placed
            };
            if fragment.placement == Placement::Float {
                fragment.placement = Placement::InFlow;
            }
            children.push(fragment);
            absolutes.extend(nested);
        }
        FlowResult {
            height: if column { cursor - style.gap } else { cross },
            margin_top: CollapsibleMargin::default(),
            margin_bottom: CollapsibleMargin::default(),
            has_content: true,
            absolutes,
        }
    }

    /// The min-content and max-content widths of a block container's content
    /// box (CSS Sizing 3 §5).
    pub(crate) fn intrinsic_content_sizes(&self, block: &BlockBox) -> (f32, f32) {
        if let Some(width) = self.control_size(block, None, None).width {
            return (width, width);
        }
        match &block.contents {
            BlockContents::Blocks(list) => list
                .iter()
                .filter(|child| !child.style.is_absolutely_positioned())
                .map(|child| self.intrinsic_outer_sizes(child))
                .fold((0.0_f32, 0.0_f32), |(min, max), (child_min, child_max)| {
                    (min.max(child_min), max.max(child_max))
                }),
            BlockContents::Inlines(items) => self.inline_intrinsic_sizes(items, &block.style),
            BlockContents::Replaced(replaced) => {
                // Percentages and `auto` resolve against an indefinite
                // containing block when measuring intrinsic sizes.
                let width = replaced.used_size(&block.style, None, None).0;
                (width, width)
            }
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
        let outer = style.borders.border_width.horizontal()
            + fixed(style.padding.left)
            + fixed(style.padding.right)
            + fixed(style.margin.left)
            + fixed(style.margin.right);
        let (min, max) = match style.width {
            Dimension::Px(_) => {
                let width = style.content_width(None).unwrap_or(0.0);
                (width, width)
            }
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
    /// Absolutely positioned boxes found in the content, relative to the
    /// content-box origin.
    absolutes: Vec<PendingAbsolute>,
}

/// The fragment placement for a box with `style`.
fn placement(style: &ComputedStyle) -> Placement {
    match style.position {
        Position::Relative => Placement::Relative,
        Position::Sticky => Placement::Sticky,
        Position::Absolute => Placement::Absolute,
        Position::Fixed => Placement::Fixed,
        Position::Static if style.is_floated() => Placement::Float,
        Position::Static => Placement::InFlow,
    }
}

/// Clips every text run in `lines`, including those of nested inline
/// boxes, to `clip`, intersecting with any clip a run already has.
fn clip_text(lines: &mut [LineBox], clip: Rect) {
    for line in lines {
        for fragment in &mut line.fragments {
            if let LineFragment::Text(text) = fragment {
                text.clip = Some(match text.clip {
                    Some(inner) => intersect(inner, clip),
                    None => clip,
                });
            }
        }
    }
}

/// The overlap of two rectangles; empty (zero-sized) when they are apart.
fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    Rect {
        x,
        y,
        width: (a.right().min(b.right()) - x).max(0.0),
        height: (a.bottom().min(b.bottom()) - y).max(0.0),
    }
}

pub(crate) fn box_style(style: &ComputedStyle) -> BoxStyle {
    BoxStyle {
        background_color: style.background_color,
        text_color: style.color,
        border_width: style.borders.border_width,
        border_color: style.borders.border_color,
        border_style: style.borders.border_style.clone(),
        border_radius: style.border_radius,
        box_shadow: Arc::clone(&style.box_shadow),
    }
}
