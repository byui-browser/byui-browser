//! Inline formatting: white-space processing (CSS Text 3 §4), line breaking
//! (CSS Text 3 §5), and line box construction (CSS 2.2 §9.4.2, §10.8).
//!
//! Supported today: `white-space: normal` collapsing, soft wrap
//! opportunities at spaces and around atomic inlines, forced breaks from
//! `<br>`, inline boxes split across lines, `line-height: normal`,
//! `vertical-align: baseline`, and left-aligned left-to-right text. Other
//! `white-space`, `text-align`, `line-height`, and `vertical-align` values, as
//! well as bidirectional text and hyphenation, are not implemented.

use std::ops::Range;

use html::NodeID;

use crate::Rect;
use crate::block::BlockResult;
use crate::box_tree::{BlockBox, InlineItem, is_document_white_space};
use crate::layout::Context;
use crate::style::ComputedStyle;
use crate::tree::{InlineBoxFragment, LayoutBox, LineBox, LineFragment, TextFragment};

/// Tolerance for floating-point error when testing whether content fits.
const FIT_EPSILON: f32 = 0.001;

/// One unit of inline content for line breaking.
#[derive(Debug)]
enum Piece {
    /// A run of non-space characters from `TextSource` `source`.
    Word {
        source: usize,
        text: String,
        width: f32,
    },
    /// A collapsible space that survived white-space collapsing. A soft wrap
    /// opportunity follows it.
    Space { source: usize, width: f32 },
    /// The start of inline box `boxes[index]`.
    Start(usize),
    /// The end of inline box `boxes[index]`.
    End(usize),
    /// Atomic inline `atomics[index]`.
    Atomic(usize),
    /// A forced line break.
    ForcedBreak,
}

struct TextSource<'a> {
    node: NodeID,
    element: Option<NodeID>,
    style: &'a ComputedStyle,
}

/// An inline box with its horizontal edges resolved. Start-side values are
/// zero when the box was split before this point; end-side values likewise.
struct InlineBoxInfo<'a> {
    node: NodeID,
    style: &'a ComputedStyle,
    margin_start: f32,
    padding_start: f32,
    padding_end: f32,
    margin_end: f32,
    padding_top: f32,
    padding_bottom: f32,
}

struct AtomicInfo {
    /// The laid-out box; `None` when only measuring intrinsic sizes.
    result: Option<BlockResult>,
    /// Margin-box width used for line breaking (the max-content
    /// contribution when measuring).
    width: f32,
    /// The min-content contribution.
    min_width: f32,
}

struct Prepared<'a> {
    pieces: Vec<Piece>,
    sources: Vec<TextSource<'a>>,
    boxes: Vec<InlineBoxInfo<'a>>,
    atomics: Vec<AtomicInfo>,
}

/// Content between two soft wrap opportunities; it cannot be split.
#[derive(Debug)]
struct Chunk {
    pieces: Range<usize>,
    width: f32,
    min_width: f32,
    /// Width of collapsible spaces at the chunk's end, which are removed if
    /// the chunk ends a line.
    trailing_space: f32,
    forced_break: bool,
}

impl Chunk {
    fn starting_at(index: usize) -> Self {
        Self {
            pieces: index..index,
            width: 0.0,
            min_width: 0.0,
            trailing_space: 0.0,
            forced_break: false,
        }
    }
}

impl Context<'_> {
    /// Lays out an inline formatting context into line boxes `available_width`
    /// wide, stacked from y = 0 in the container's content-box coordinates.
    ///
    /// `container` provides the strut: every non-empty line is at least as
    /// tall as the container's font and line height.
    pub(crate) fn layout_inline(
        &self,
        items: &[InlineItem],
        container: &ComputedStyle,
        available_width: f32,
    ) -> Vec<LineBox> {
        let mut prepared = self.prepare(items, Some(available_width));
        let chunks = chunk(&prepared);
        let mut lines = Vec::new();
        let mut open = Vec::new();
        let mut y = 0.0;
        for range in break_lines(&chunks, available_width) {
            if let Some(line) = self.build_line(
                range,
                &mut prepared,
                &mut open,
                container,
                available_width,
                y,
            ) {
                y = line.rect.bottom();
                lines.push(line);
            }
        }
        lines
    }

    /// The min-content and max-content widths of an inline formatting
    /// context (CSS Sizing 3 §5.1).
    pub(crate) fn inline_intrinsic_sizes(
        &self,
        items: &[InlineItem],
        _container: &ComputedStyle,
    ) -> (f32, f32) {
        let prepared = self.prepare(items, None);
        let mut min = 0.0_f32;
        let mut max = 0.0_f32;
        let mut line = 0.0_f32;
        for chunk in chunk(&prepared) {
            min = min.max(chunk.min_width - chunk.trailing_space);
            line += chunk.width;
            max = max.max(line - chunk.trailing_space);
            if chunk.forced_break {
                line = 0.0;
            }
        }
        (min, max)
    }

    /// Converts inline items to pieces, collapsing white space
    /// (CSS Text 3 §4.1.1): every sequence of spaces, tabs, and segment breaks
    /// becomes one space, and a space following another collapsible space, even
    /// across inline box boundaries, is removed. Spaces at the start of the
    /// formatting context and after a forced break are removed as line-start
    /// spaces (§4.1.2).
    ///
    /// Atomic inlines are laid out when `cb_width` is given and measured
    /// otherwise.
    fn prepare<'a>(&self, items: &'a [InlineItem], cb_width: Option<f32>) -> Prepared<'a> {
        let mut prepared = Prepared {
            pieces: Vec::new(),
            sources: Vec::new(),
            boxes: Vec::new(),
            atomics: Vec::new(),
        };
        let mut after_space = true;
        let mut stack = Vec::new();
        for item in items {
            match item {
                InlineItem::Text {
                    node,
                    element,
                    text,
                    style,
                } => {
                    let source = prepared.sources.len();
                    prepared.sources.push(TextSource {
                        node: *node,
                        element: *element,
                        style,
                    });
                    let mut rest = text.as_str();
                    while let Some(first) = rest.chars().next() {
                        let is_space = is_document_white_space(first);
                        let end = rest
                            .find(|character| is_document_white_space(character) != is_space)
                            .unwrap_or(rest.len());
                        let (run, remaining) = rest.split_at(end);
                        rest = remaining;
                        if is_space {
                            if !after_space {
                                prepared.pieces.push(Piece::Space {
                                    source,
                                    width: self.fonts.text_width(" ", style.font_size),
                                });
                                after_space = true;
                            }
                        } else {
                            prepared.pieces.push(Piece::Word {
                                source,
                                text: run.to_owned(),
                                width: self.fonts.text_width(run, style.font_size),
                            });
                            after_space = false;
                        }
                    }
                }
                InlineItem::Start {
                    node,
                    style,
                    start_edge,
                } => {
                    let edge = |value: crate::style::Dimension| {
                        if *start_edge {
                            value.resolve_or_zero(cb_width)
                        } else {
                            0.0
                        }
                    };
                    stack.push(prepared.boxes.len());
                    prepared.pieces.push(Piece::Start(prepared.boxes.len()));
                    prepared.boxes.push(InlineBoxInfo {
                        node: *node,
                        style,
                        margin_start: edge(style.margin.left),
                        padding_start: edge(style.padding.left),
                        padding_end: 0.0,
                        margin_end: 0.0,
                        padding_top: style.padding.top.resolve_or_zero(cb_width),
                        padding_bottom: style.padding.bottom.resolve_or_zero(cb_width),
                    });
                }
                InlineItem::End { end_edge } => {
                    let Some(index) = stack.pop() else {
                        continue;
                    };
                    let info = &mut prepared.boxes[index];
                    if *end_edge {
                        info.padding_end = info.style.padding.right.resolve_or_zero(cb_width);
                        info.margin_end = info.style.margin.right.resolve_or_zero(cb_width);
                    }
                    prepared.pieces.push(Piece::End(index));
                }
                InlineItem::Atomic(block) => {
                    prepared.pieces.push(Piece::Atomic(prepared.atomics.len()));
                    prepared.atomics.push(self.atomic(block, cb_width));
                    after_space = false;
                }
                InlineItem::ForcedBreak => {
                    prepared.pieces.push(Piece::ForcedBreak);
                    after_space = true;
                }
            }
        }
        prepared
    }

    fn atomic(&self, block: &BlockBox, cb_width: Option<f32>) -> AtomicInfo {
        match cb_width {
            Some(cb_width) => {
                let result = self.layout_inline_block(block, cb_width);
                let fragment = &result.fragment;
                let width = fragment.margin.left + fragment.rect.width + fragment.margin.right;
                AtomicInfo {
                    result: Some(result),
                    width,
                    min_width: width,
                }
            }
            None => {
                let (min_width, width) = self.intrinsic_outer_sizes(block);
                AtomicInfo {
                    result: None,
                    width,
                    min_width,
                }
            }
        }
    }

    /// Places one line's pieces and computes its height by aligning baselines
    /// (CSS 2.2 §10.8). Returns `None` for a line with no content, which is
    /// treated as zero-height and does not exist for layout (§9.4.2).
    ///
    /// `open` holds the inline boxes still open from earlier lines; they
    /// continue on this line without their start edge.
    fn build_line(
        &self,
        range: Range<usize>,
        prepared: &mut Prepared<'_>,
        open: &mut Vec<usize>,
        container: &ComputedStyle,
        available_width: f32,
        y: f32,
    ) -> Option<LineBox> {
        let is_content =
            |piece: &Piece| !matches!(piece, Piece::Start(_) | Piece::End(_) | Piece::Space { .. });
        let first_content = range
            .clone()
            .find(|index| is_content(&prepared.pieces[*index]))
            .unwrap_or(range.end);
        let last_content = range
            .clone()
            .rev()
            .find(|index| is_content(&prepared.pieces[*index]))
            .map_or(range.start, |index| index + 1);
        // Collapsible spaces at the start and end of a line are removed
        // (CSS Text 3 §4.1.2).
        let kept = |index: usize| {
            !matches!(prepared.pieces[index], Piece::Space { .. })
                || (first_content..last_content).contains(&index)
        };

        let mut fragments = Vec::new();
        // (box index, fragment index, start x), innermost last.
        let mut active: Vec<(usize, usize, f32)> = Vec::new();
        let mut atomics_on_line: Vec<(usize, usize)> = Vec::new();
        let mut boxes_on_line = Vec::new();
        let mut has_content = false;
        let mut x = 0.0_f32;
        for &index in open.iter() {
            active.push((index, fragments.len(), x));
            boxes_on_line.push((index, fragments.len()));
            fragments.push(LineFragment::InlineBox(
                self.placeholder(&prepared.boxes[index]),
            ));
        }
        for index in range.filter(|index| kept(*index)) {
            match &prepared.pieces[index] {
                Piece::Start(box_index) => {
                    let info = &prepared.boxes[*box_index];
                    has_content |= info.margin_start != 0.0 || info.padding_start != 0.0;
                    x += info.margin_start;
                    active.push((*box_index, fragments.len(), x));
                    boxes_on_line.push((*box_index, fragments.len()));
                    fragments.push(LineFragment::InlineBox(self.placeholder(info)));
                    x += info.padding_start;
                    open.push(*box_index);
                }
                Piece::End(box_index) => {
                    let info = &prepared.boxes[*box_index];
                    has_content |= info.margin_end != 0.0 || info.padding_end != 0.0;
                    x += info.padding_end;
                    if let Some((_, fragment, start)) = active.pop() {
                        set_horizontal_extent(&mut fragments[fragment], start, x);
                    }
                    x += info.margin_end;
                    open.pop();
                }
                Piece::Word {
                    source,
                    text,
                    width,
                } => {
                    has_content = true;
                    self.push_text(&mut fragments, &prepared.sources[*source], text, x, *width);
                    x += width;
                }
                Piece::Space { source, width } => {
                    self.push_text(&mut fragments, &prepared.sources[*source], " ", x, *width);
                    x += width;
                }
                Piece::Atomic(atomic_index) => {
                    has_content = true;
                    atomics_on_line.push((*atomic_index, fragments.len()));
                    let info = &mut prepared.atomics[*atomic_index];
                    let mut fragment = info
                        .result
                        .take()
                        .expect("atomic inlines are laid out once")
                        .fragment;
                    fragment.translate(x + fragment.margin.left, 0.0);
                    fragments.push(LineFragment::Atomic(fragment));
                    x += info.width;
                }
                Piece::ForcedBreak => has_content = true,
            }
        }
        for (_, fragment, start) in active.drain(..) {
            set_horizontal_extent(&mut fragments[fragment], start, x);
        }
        if !has_content {
            return None;
        }

        // Baseline alignment: each inline box and the strut contribute their
        // half-leading-adjusted ascent and descent (CSS 2.2 §10.8.1).
        let leaded = |style: &ComputedStyle| {
            let metrics = self.fonts.vertical_metrics(style.font_size);
            let line_height = metrics.ascent + metrics.descent + metrics.line_gap;
            let half_leading = (line_height - metrics.ascent - metrics.descent) / 2.0;
            (
                metrics.ascent + half_leading,
                metrics.descent + half_leading,
            )
        };
        let (mut above, mut below) = leaded(container);
        let mut include = |(a, b): (f32, f32)| {
            above = above.max(a);
            below = below.max(b);
        };
        for (box_index, _) in &boxes_on_line {
            include(leaded(prepared.boxes[*box_index].style));
        }
        for fragment in &fragments {
            if let LineFragment::Text(text) = fragment {
                let metrics = self.fonts.vertical_metrics(text.font_size);
                let half_leading = metrics.line_gap / 2.0;
                include((
                    metrics.ascent + half_leading,
                    metrics.descent + half_leading,
                ));
            }
        }
        let atomic_ascents = atomics_on_line
            .iter()
            .map(|(_, fragment)| {
                let LineFragment::Atomic(atomic) = &fragments[*fragment] else {
                    unreachable!("atomic indices point at atomic fragments");
                };
                let margin_box_height =
                    atomic.margin.top + atomic.rect.height + atomic.margin.bottom;
                let ascent = atomic.margin.top
                    + last_baseline(atomic).unwrap_or(atomic.rect.height + atomic.margin.bottom);
                (ascent, margin_box_height - ascent)
            })
            .collect::<Vec<_>>();
        for extent in &atomic_ascents {
            include(*extent);
        }
        let baseline = y + above;

        for fragment in &mut fragments {
            if let LineFragment::Text(text) = fragment {
                let metrics = self.fonts.vertical_metrics(text.font_size);
                text.baseline = baseline;
                text.rect.y = baseline - metrics.ascent;
                text.rect.height = metrics.ascent + metrics.descent;
            }
        }
        for (box_index, fragment) in boxes_on_line {
            let info = &prepared.boxes[box_index];
            let metrics = self.fonts.vertical_metrics(info.style.font_size);
            if let LineFragment::InlineBox(inline) = &mut fragments[fragment] {
                inline.rect.y = baseline - metrics.ascent - info.padding_top;
                inline.rect.height =
                    metrics.ascent + metrics.descent + info.padding_top + info.padding_bottom;
            }
        }
        for ((_, fragment), (ascent, _)) in atomics_on_line.iter().zip(atomic_ascents) {
            if let LineFragment::Atomic(atomic) = &mut fragments[*fragment] {
                let top = baseline - ascent + atomic.margin.top;
                atomic.translate(0.0, top);
            }
        }
        Some(LineBox {
            rect: Rect {
                x: 0.0,
                y,
                width: available_width,
                height: above + below,
            },
            baseline,
            fragments,
        })
    }

    fn placeholder(&self, info: &InlineBoxInfo<'_>) -> InlineBoxFragment {
        InlineBoxFragment {
            node: info.node,
            rect: Rect::default(),
            style: crate::block::box_style(info.style),
        }
    }

    /// Appends text, merging with the previous fragment when it is the
    /// adjacent run of the same text node.
    fn push_text(
        &self,
        fragments: &mut Vec<LineFragment>,
        source: &TextSource<'_>,
        text: &str,
        x: f32,
        width: f32,
    ) {
        if let Some(LineFragment::Text(previous)) = fragments.last_mut() {
            if previous.node == source.node && (previous.rect.right() - x).abs() < FIT_EPSILON {
                previous.text.push_str(text);
                previous.rect.width += width;
                return;
            }
        }
        fragments.push(LineFragment::Text(TextFragment {
            node: source.node,
            element: source.element,
            rect: Rect {
                x,
                y: 0.0,
                width,
                height: 0.0,
            },
            baseline: 0.0,
            text: text.to_owned(),
            font_size: source.style.font_size,
            color: source.style.color,
        }));
    }
}

fn set_horizontal_extent(fragment: &mut LineFragment, start: f32, end: f32) {
    if let LineFragment::InlineBox(inline) = fragment {
        inline.rect.x = start;
        inline.rect.width = end - start;
    }
}

/// Groups pieces into unbreakable chunks. Soft wrap opportunities follow
/// spaces and surround atomic inlines; inline box starts attach to the content
/// after them and ends to the content before them.
fn chunk(prepared: &Prepared<'_>) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut current = Chunk::starting_at(0);
    let mut opportunity = false;
    for (index, piece) in prepared.pieces.iter().enumerate() {
        let (width, min_width) = match piece {
            Piece::Word { width, .. } | Piece::Space { width, .. } => (*width, *width),
            Piece::Start(box_index) => {
                let info = &prepared.boxes[*box_index];
                let width = info.margin_start + info.padding_start;
                (width, width)
            }
            Piece::End(box_index) => {
                let info = &prepared.boxes[*box_index];
                let width = info.padding_end + info.margin_end;
                (width, width)
            }
            Piece::Atomic(atomic) => {
                let info = &prepared.atomics[*atomic];
                (info.width, info.min_width)
            }
            Piece::ForcedBreak => (0.0, 0.0),
        };
        let starts_content = matches!(
            piece,
            Piece::Start(_) | Piece::Word { .. } | Piece::Atomic(_)
        );
        let break_before = opportunity || matches!(piece, Piece::Atomic(_));
        if starts_content && break_before && !current.pieces.is_empty() {
            let next = Chunk::starting_at(index);
            chunks.push(std::mem::replace(&mut current, next));
        }
        if starts_content {
            opportunity = matches!(piece, Piece::Atomic(_));
        }
        current.pieces.end = index + 1;
        current.width += width;
        current.min_width += min_width;
        match piece {
            Piece::Space { .. } => {
                current.trailing_space += width;
                opportunity = true;
            }
            Piece::Word { .. } | Piece::Atomic(_) => current.trailing_space = 0.0,
            Piece::ForcedBreak => {
                current.forced_break = true;
                current.trailing_space = 0.0;
                chunks.push(std::mem::replace(
                    &mut current,
                    Chunk::starting_at(index + 1),
                ));
                opportunity = false;
            }
            Piece::Start(_) | Piece::End(_) => {}
        }
    }
    if !current.pieces.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Greedy line breaking: a chunk moves to a new line when it would overflow a
/// non-empty line. Trailing spaces do not count toward fitting because they
/// are removed at the end of a line. A chunk wider than the line overflows it.
fn break_lines(chunks: &[Chunk], available_width: f32) -> Vec<Range<usize>> {
    let mut lines = Vec::new();
    let mut line: Option<Range<usize>> = None;
    let mut width = 0.0;
    for chunk in chunks {
        if let Some(current) = &line {
            if width + chunk.width - chunk.trailing_space > available_width + FIT_EPSILON {
                lines.push(current.clone());
                line = None;
                width = 0.0;
            }
        }
        let current = line.get_or_insert(chunk.pieces.start..chunk.pieces.start);
        current.end = chunk.pieces.end;
        width += chunk.width;
        if chunk.forced_break {
            lines.extend(line.take());
            width = 0.0;
        }
    }
    lines.extend(line);
    lines
}

/// The baseline of the last line box inside a box, relative to the same
/// origin as the box's fragments (CSS 2.2 §10.8.1 for inline-blocks).
fn last_baseline(layout_box: &LayoutBox) -> Option<f32> {
    layout_box
        .children
        .iter()
        .rev()
        .find_map(last_baseline)
        .or_else(|| layout_box.lines.last().map(|line| line.baseline))
}
