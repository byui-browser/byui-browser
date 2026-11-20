//! Inline formatting: white-space processing (CSS Text 3 §4), line breaking
//! (CSS Text 3 §5), and line box construction (CSS 2.2 §9.4.2, §10.8).
//!
//! Supported today: `white-space: normal` collapsing, soft wrap
//! opportunities at spaces and around atomic inlines, forced breaks from
//! `<br>`, inline boxes split across lines, `line-height`,
//! `vertical-align: baseline`, preserved unbreakable runs for text the user
//! agent draws in form controls, and left-to-right text aligned by every
//! `text-align` keyword (`justify` widens the spaces between words). Other
//! non-`normal` `white-space` and `vertical-align` values, as well as
//! bidirectional text, `text-align-last`, and hyphenation, are not
//! implemented.
//!
//! Line boxes are shortened by floats beside them (CSS 2.2 §9.5). A float
//! met in the middle of a line is placed beside that line when it fits next
//! to the content already on it, and below the line otherwise.

use std::ops::Range;

use html::NodeID;
use unicode_segmentation::UnicodeSegmentation;

use crate::Rect;
use crate::block::{BlockResult, ContainingBlock};
use crate::box_tree::{BlockBox, InlineItem, WhiteSpace, is_document_white_space};
use crate::layout::Context;
use crate::positioned::{FloatScope, PendingAbsolute, relative_offset};
use crate::style::{ComputedStyle, Position, TextAlign};
use crate::tree::{InlineBoxFragment, LayoutBox, LineBox, LineFragment, Placement, TextFragment};

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
    /// White space followed by a soft wrap opportunity. With `text: None`
    /// it is a collapsible space that survived white-space collapsing,
    /// removed at the start and end of a line. With `Some` it is a
    /// preserved run (`pre-wrap`) drawn as written, kept at the start of a
    /// line and hanging (not drawn) at its end.
    Space {
        source: usize,
        width: f32,
        text: Option<String>,
    },
    /// The start of inline box `boxes[index]`.
    Start(usize),
    /// The end of inline box `boxes[index]`.
    End(usize),
    /// Atomic inline `atomics[index]`.
    Atomic(usize),
    /// A forced line break.
    ForcedBreak,
    /// Floated or absolutely positioned box `outs[index]`; it takes no room
    /// in the line.
    OutOfFlow(usize),
}

/// The result of laying out an inline formatting context, in the
/// container's content-box coordinates.
pub(crate) struct InlineResult {
    pub lines: Vec<LineBox>,
    /// Floats placed while laying out the lines.
    pub floats: Vec<LayoutBox>,
    /// Absolutely positioned boxes still looking for their containing block.
    pub absolutes: Vec<PendingAbsolute>,
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
    outs: Vec<&'a BlockBox>,
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
    ///
    /// Lines are broken one at a time (greedily) into the space `floats`
    /// leaves at their height, estimated as the strut's height. A line whose
    /// first unbreakable chunk does not fit beside floats moves down below
    /// the float that is in the way.
    pub(crate) fn layout_inline(
        &self,
        items: &[InlineItem],
        container: &ComputedStyle,
        available_width: f32,
        floats: &mut FloatScope<'_>,
    ) -> InlineResult {
        let mut prepared = self.prepare(items, Some(available_width));
        let chunks = chunk(&prepared);
        let mut result = InlineResult {
            lines: Vec::new(),
            floats: Vec::new(),
            absolutes: Vec::new(),
        };
        let strut = {
            let metrics = self
                .fonts
                .font_vertical_metrics(container.font, container.font_size);
            metrics.ascent + metrics.descent + metrics.line_gap
        };
        let mut placed = vec![false; prepared.outs.len()];
        let mut open = Vec::new();
        let mut y = 0.0;
        let mut next = 0;
        while next < chunks.len() {
            let (mut band_x, mut band_width) = floats.line_band(y, strut, available_width);
            let mut width = 0.0_f32;
            let mut end = next;
            while end < chunks.len() {
                let chunk = &chunks[end];
                if end > next
                    && width + chunk.width - chunk.trailing_space > band_width + FIT_EPSILON
                {
                    break;
                }
                for index in chunk.pieces.clone() {
                    let Piece::OutOfFlow(out) = prepared.pieces[index] else {
                        continue;
                    };
                    let block = prepared.outs[out];
                    if placed[out] || !block.style.is_floated() {
                        continue;
                    }
                    placed[out] = true;
                    // A float goes beside the current line if it fits next
                    // to the content already there, and below it otherwise.
                    let (min, _) = self.intrinsic_outer_sizes(block);
                    let min_y = if width + min <= band_width + FIT_EPSILON {
                        y
                    } else {
                        y + strut
                    };
                    let (fragment, nested) =
                        self.layout_float(block, available_width, floats, min_y);
                    result.floats.push(fragment);
                    result.absolutes.extend(nested);
                    (band_x, band_width) = floats.line_band(y, strut, available_width);
                }
                width += chunk.width;
                end += 1;
                if chunk.forced_break {
                    break;
                }
            }
            let first = &chunks[next];
            if end == next + 1
                && first.width - first.trailing_space > band_width + FIT_EPSILON
                && band_width + FIT_EPSILON < available_width
                && let Some(below) = floats.next_line_edge(y, strut, available_width)
            {
                log::trace!("line at y {y} does not fit beside floats; moving to y {below}");
                y = below;
                continue;
            }
            let range = first.pieces.start..chunks[end - 1].pieces.end;
            next = end;
            log::trace!("line break: pieces {}..{}", range.start, range.end);
            // `justify` leaves the last line and lines ended by a forced
            // break start-aligned (CSS Text 3 §7.2, `text-align-last: auto`).
            let ends_paragraph = next == chunks.len()
                || range
                    .clone()
                    .rev()
                    .any(|index| matches!(prepared.pieces[index], Piece::ForcedBreak));
            let align = match container.text_align {
                TextAlign::Justify if ends_paragraph => TextAlign::Start,
                align => align,
            };
            let mut absolutes = Vec::new();
            let line = self.build_line(
                range,
                &mut prepared,
                &mut open,
                container,
                align,
                band_width,
                y,
                &mut absolutes,
            );
            for mut absolute in absolutes {
                absolute.translate(band_x, 0.0);
                result.absolutes.push(absolute);
            }
            if let Some(mut line) = line {
                line.translate(band_x, 0.0);
                log::trace!(
                    "line {} at y {}: height {}, baseline {}, {} fragments",
                    result.lines.len(),
                    line.rect.y,
                    line.rect.height,
                    line.baseline,
                    line.fragments.len()
                );
                y = line.rect.bottom();
                result.lines.push(line);
            }
        }
        result
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
        // Floats sit beside the lines, so at most they all fit side by side.
        let mut floats = 0.0_f32;
        for block in prepared
            .outs
            .iter()
            .filter(|block| block.style.is_floated())
        {
            let (float_min, float_max) = self.intrinsic_outer_sizes(block);
            min = min.max(float_min);
            floats += float_max;
        }
        for chunk in chunk(&prepared) {
            min = min.max(chunk.min_width - chunk.trailing_space);
            line += chunk.width;
            max = max.max(line - chunk.trailing_space);
            if chunk.forced_break {
                line = 0.0;
            }
        }
        (min, max + floats)
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
            outs: Vec::new(),
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
                    white_space,
                } => {
                    let source = prepared.sources.len();
                    prepared.sources.push(TextSource {
                        node: *node,
                        element: *element,
                        style,
                    });
                    if *white_space == WhiteSpace::PreWrap {
                        for (is_space, run) in preserved_runs(text) {
                            // Tab stops are not supported: a tab is one space.
                            let run = if is_space {
                                run.replace('\t', " ")
                            } else {
                                run.to_owned()
                            };
                            let run = run.as_str();
                            let width =
                                self.fonts.font_text_width(style.font, run, style.font_size);
                            prepared.pieces.push(if is_space {
                                Piece::Space {
                                    source,
                                    width,
                                    text: Some(run.to_owned()),
                                }
                            } else {
                                Piece::Word {
                                    source,
                                    text: run.to_owned(),
                                    width,
                                }
                            });
                        }
                        after_space = false;
                        continue;
                    }
                    if *white_space == WhiteSpace::Pre || style.nowrap {
                        // One unbreakable run with its spaces kept.
                        if !text.is_empty() {
                            let text = if style.nowrap {
                                text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
                            } else {
                                text.clone()
                            };
                            let width =
                                self.fonts
                                    .font_text_width(style.font, &text, style.font_size);
                            prepared.pieces.push(Piece::Word {
                                source,
                                text,
                                width,
                            });
                            after_space = false;
                        }
                        continue;
                    }
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
                                    width: self.fonts.font_text_width(
                                        style.font,
                                        " ",
                                        style.font_size,
                                    ),
                                    text: None,
                                });
                                after_space = true;
                            }
                        } else {
                            let break_all = style.word_break == "break-all"
                                || (style.word_break == "break-word"
                                    && cb_width.is_some_and(|width| {
                                        self.fonts.font_text_width(style.font, run, style.font_size)
                                            > width
                                    }));
                            let mut start = 0;
                            for (offset, grapheme) in run.grapheme_indices(true) {
                                let character = grapheme.chars().next().expect("nonempty grapheme");
                                // Basic CJK opportunities. Full Unicode line breaking and
                                // dictionary-based Southeast Asian breaking are not implemented.
                                let cjk = matches!(character as u32, 0x2e80..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff);
                                if offset > start
                                    && (break_all || (style.word_break != "keep-all" && cjk))
                                {
                                    let part = &run[start..offset];
                                    prepared.pieces.push(Piece::Word {
                                        source,
                                        text: part.to_owned(),
                                        width: self.fonts.font_text_width(
                                            style.font,
                                            part,
                                            style.font_size,
                                        ),
                                    });
                                    prepared.pieces.push(Piece::Space {
                                        source,
                                        width: 0.0,
                                        text: None,
                                    });
                                    start = offset;
                                }
                            }
                            let part = &run[start..];
                            prepared.pieces.push(Piece::Word {
                                source,
                                text: part.to_owned(),
                                width: self.fonts.font_text_width(
                                    style.font,
                                    part,
                                    style.font_size,
                                ),
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
                        padding_start: (edge(style.padding.left) + style.borders.border_width.left),
                        padding_end: 0.0,
                        margin_end: 0.0,
                        padding_top: (style.padding.top.resolve_or_zero(cb_width)
                            + style.borders.border_width.top),
                        padding_bottom: (style.padding.bottom.resolve_or_zero(cb_width)
                            + style.borders.border_width.bottom),
                    });
                }
                InlineItem::End { end_edge } => {
                    let Some(index) = stack.pop() else {
                        continue;
                    };
                    let info = &mut prepared.boxes[index];
                    if *end_edge {
                        info.padding_end = info.style.padding.right.resolve_or_zero(cb_width)
                            + info.style.borders.border_width.right;
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
                // Out-of-flow boxes do not affect white-space collapsing.
                InlineItem::OutOfFlow(block) => {
                    prepared.pieces.push(Piece::OutOfFlow(prepared.outs.len()));
                    prepared.outs.push(block);
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
    ///
    /// `align` places the content horizontally (CSS Text 3 §7.1). Content
    /// wider than the line is start-aligned. For `justify`, the free space is
    /// shared equally by the line's spaces; a text run is split after each
    /// widened space so its glyphs stay where layout put them.
    ///
    /// Absolutely positioned boxes in the range are pushed onto `absolutes`
    /// at their static position, as are those of atomic inlines. A box that
    /// was inline-level sits where it occurs in the line; one that was
    /// block-level sits below the line if content precedes it.
    #[allow(clippy::too_many_arguments)]
    fn build_line(
        &self,
        range: Range<usize>,
        prepared: &mut Prepared<'_>,
        open: &mut Vec<usize>,
        container: &ComputedStyle,
        align: TextAlign,
        available_width: f32,
        y: f32,
        absolutes: &mut Vec<PendingAbsolute>,
    ) -> Option<LineBox> {
        let is_content = |piece: &Piece| {
            !matches!(
                piece,
                Piece::Start(_) | Piece::End(_) | Piece::Space { .. } | Piece::OutOfFlow(_)
            )
        };
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
        // Preserved spaces are kept at the start but hang at the end.
        let kept = |index: usize| match prepared.pieces[index] {
            Piece::Space { text: None, .. } => (first_content..last_content).contains(&index),
            Piece::Space { text: Some(_), .. } => index < last_content,
            _ => true,
        };

        // The width of the line's content before alignment, and how many
        // spaces `justify` can widen.
        let (natural_width, spaces) = range.clone().filter(|index| kept(*index)).fold(
            (0.0_f32, 0_u32),
            |(width, spaces), index| match &prepared.pieces[index] {
                Piece::Start(box_index) => {
                    let info = &prepared.boxes[*box_index];
                    (width + info.margin_start + info.padding_start, spaces)
                }
                Piece::End(box_index) => {
                    let info = &prepared.boxes[*box_index];
                    (width + info.padding_end + info.margin_end, spaces)
                }
                Piece::Word { width: word, .. } => (width + word, spaces),
                Piece::Space { width: space, .. } => {
                    (width + space, spaces + u32::from(*space > 0.0))
                }
                Piece::Atomic(atomic_index) => {
                    (width + prepared.atomics[*atomic_index].width, spaces)
                }
                Piece::ForcedBreak | Piece::OutOfFlow(_) => (width, spaces),
            },
        );
        let free_space = (available_width - natural_width).max(0.0);
        let space_extra = if align == TextAlign::Justify && spaces > 0 {
            free_space / spaces as f32
        } else {
            0.0
        };
        let offset = if align == TextAlign::Justify {
            0.0
        } else {
            free_space * align.offset_fraction()
        };

        let mut fragments = Vec::new();
        // (box index, fragment index, start x), innermost last.
        let mut active: Vec<(usize, usize, f32)> = Vec::new();
        let mut atomics_on_line: Vec<(usize, usize)> = Vec::new();
        // Relative offsets and pending absolutes of the line's atomics, by
        // fragment index.
        let mut atomic_extras: Vec<(usize, (f32, f32), Vec<PendingAbsolute>)> = Vec::new();
        // (box index, first fragment, end fragment) of each inline box part.
        let mut box_ranges: Vec<(usize, usize, usize)> = Vec::new();
        // (out index, x, follows content) of absolutely positioned boxes.
        let mut statics: Vec<(usize, f32, bool)> = Vec::new();
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
                    if let Some((index, fragment, start)) = active.pop() {
                        set_horizontal_extent(&mut fragments[fragment], start, x);
                        box_ranges.push((index, fragment, fragments.len()));
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
                Piece::Space {
                    source,
                    width,
                    text,
                } => {
                    let text = text.as_deref().unwrap_or(" ");
                    self.push_text(&mut fragments, &prepared.sources[*source], text, x, *width);
                    // The gap after a widened space keeps the next word out
                    // of this text run.
                    x += width + space_extra;
                }
                Piece::Atomic(atomic_index) => {
                    has_content = true;
                    atomics_on_line.push((*atomic_index, fragments.len()));
                    let info = &mut prepared.atomics[*atomic_index];
                    let result = info
                        .result
                        .take()
                        .expect("atomic inlines are laid out once");
                    atomic_extras.push((fragments.len(), result.relative, result.absolutes));
                    let mut fragment = result.fragment;
                    fragment.translate(x + fragment.margin.left, 0.0);
                    fragments.push(LineFragment::Atomic(fragment));
                    x += info.width;
                }
                Piece::ForcedBreak => has_content = true,
                Piece::OutOfFlow(out) => {
                    if prepared.outs[*out].style.is_absolutely_positioned() {
                        statics.push((*out, x, has_content));
                    }
                }
            }
        }
        for (index, fragment, start) in active.drain(..) {
            set_horizontal_extent(&mut fragments[fragment], start, x);
            box_ranges.push((index, fragment, fragments.len()));
        }
        let pending = |out: usize, x: f32, y: f32| PendingAbsolute {
            block: prepared.outs[out].clone(),
            x,
            y,
        };
        if !has_content {
            absolutes.extend(statics.into_iter().map(|(out, x, _)| pending(out, x, y)));
            return None;
        }
        if offset > 0.0 {
            for fragment in &mut fragments {
                match fragment {
                    LineFragment::InlineBox(inline) => inline.rect.x += offset,
                    LineFragment::Text(text) => text.rect.x += offset,
                    LineFragment::Atomic(atomic) => atomic.translate(offset, 0.0),
                }
            }
        }

        // Baseline alignment: each inline box and the strut contribute their
        // half-leading-adjusted ascent and descent (CSS 2.2 §10.8.1).
        let leaded = |style: &ComputedStyle| {
            let metrics = self
                .fonts
                .font_vertical_metrics(style.font, style.font_size);
            let normal = metrics.ascent + metrics.descent + metrics.line_gap;
            let line_height = style.line_height.used(style.font_size, normal);
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
            if let LineFragment::Text(text) = fragment
                && let Some(source) = prepared
                    .sources
                    .iter()
                    .find(|source| source.node == text.node)
            {
                include(leaded(source.style));
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
                let metrics = self.fonts.font_vertical_metrics(text.font, text.font_size);
                text.baseline = baseline;
                text.rect.y = baseline - metrics.ascent;
                text.rect.height = metrics.ascent + metrics.descent;
            }
        }
        for (box_index, fragment) in boxes_on_line {
            let info = &prepared.boxes[box_index];
            let metrics = self
                .fonts
                .font_vertical_metrics(info.style.font, info.style.font_size);
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
        for (fragment, (dx, dy), nested) in atomic_extras {
            if let LineFragment::Atomic(atomic) = &mut fragments[fragment] {
                atomic.translate(dx, dy);
                for mut absolute in nested {
                    absolute.translate(atomic.rect.x, atomic.rect.y);
                    absolutes.push(absolute);
                }
            }
        }
        // Relatively positioned inline boxes move with their content.
        let cb = ContainingBlock {
            width: available_width,
            height: None,
        };
        for (box_index, start, end) in box_ranges {
            let style = prepared.boxes[box_index].style;
            if style.vertical_align_offset != 0.0 {
                for fragment in &mut fragments[start..end] {
                    translate_fragment(fragment, 0.0, style.vertical_align_offset);
                }
            }
            if style.position != Position::Relative {
                continue;
            }
            let (dx, dy) = relative_offset(style, cb);
            for fragment in &mut fragments[start..end] {
                translate_fragment(fragment, dx, dy);
            }
        }
        let bottom = y + above + below;
        absolutes.extend(statics.into_iter().map(|(out, x, follows_content)| {
            if follows_content && !prepared.outs[out].style.blockified_from_inline {
                pending(out, 0.0, bottom)
            } else {
                pending(out, x + offset, y)
            }
        }));
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
        if let Some(LineFragment::Text(previous)) = fragments.last_mut()
            && previous.node == source.node
            && (previous.rect.right() - x).abs() < FIT_EPSILON
        {
            previous.text.push_str(text);
            previous.rect.width += width;
            return;
        }
        fragments.push(LineFragment::Text(TextFragment {
            cursor: source.style.cursor.clone(),
            user_select: source.style.user_select.clone(),
            text_rendering: source.style.text_rendering.clone(),
            hidden: source.style.hidden,
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
            font: source.style.font,
            color: source.style.color,
            text_decoration: source.style.text_decoration.clone(),
            clip: None,
        }));
    }
}

/// Moves one line fragment by `dx`, `dy`.
fn translate_fragment(fragment: &mut LineFragment, dx: f32, dy: f32) {
    match fragment {
        LineFragment::InlineBox(inline) => inline.rect = inline.rect.translate(dx, dy),
        LineFragment::Text(text) => {
            text.rect = text.rect.translate(dx, dy);
            text.baseline += dy;
        }
        LineFragment::Atomic(atomic) => atomic.translate(dx, dy),
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
            Piece::ForcedBreak | Piece::OutOfFlow(_) => (0.0, 0.0),
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
            Piece::Start(_) | Piece::End(_) | Piece::OutOfFlow(_) => {}
        }
    }
    if !current.pieces.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// The baseline of the last line box inside a box, relative to the same
/// origin as the box's fragments (CSS 2.2 §10.8.1 for inline-blocks).
fn last_baseline(layout_box: &LayoutBox) -> Option<f32> {
    layout_box
        .children
        .iter()
        .rev()
        .filter(|child| child.placement == Placement::InFlow)
        .find_map(last_baseline)
        .or_else(|| layout_box.lines.last().map(|line| line.baseline))
}

/// Splits `pre-wrap` text into alternating runs of spaces and tabs (`true`)
/// and other characters (`false`).
fn preserved_runs(text: &str) -> impl Iterator<Item = (bool, &str)> {
    let is_space = |character: char| matches!(character, ' ' | '\t');
    let mut rest = text;
    std::iter::from_fn(move || {
        let first = rest.chars().next()?;
        let space = is_space(first);
        let end = rest
            .find(|character| is_space(character) != space)
            .unwrap_or(rest.len());
        let (run, remaining) = rest.split_at(end);
        rest = remaining;
        Some((space, run))
    })
}
