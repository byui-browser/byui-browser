//! Display-list commands.

use crate::{Color, CornerRadii, FontId, ImageKey, PaintBox, Rect};

/// One ordered drawing command. Coordinates are CSS pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    /// Fills a rectangle with an RGBA color.
    FillRect {
        /// Rectangle to fill.
        rect: Rect,
        /// Opaque or translucent fill color.
        color: Color,
    },
    /// Fills a rectangle with elliptical corners with an RGBA color.
    ///
    /// Painted for backgrounds of boxes with a non-zero `border-radius`.
    FillRoundedRect {
        /// Bounding rectangle to fill.
        rect: Rect,
        /// Corner radii in CSS pixels, already reduced to fit `rect`.
        radii: CornerRadii,
        /// Opaque or translucent fill color.
        color: Color,
    },
    /// Draws a run of text.
    Text {
        /// Left edge of the first glyph's advance.
        x: f32,
        /// The alphabetic baseline the glyphs sit on.
        baseline: f32,
        /// Font size; the renderer scales its glyphs to it.
        font_size: f32,
        /// The face layout measured the run with
        /// ([`layout::TextFragment::font`]); [`FontId::DEFAULT`] for the
        /// renderer's default font.
        font: FontId,
        /// Text to draw.
        text: String,
        /// Text color.
        color: Color,
    },
    /// Draws a picture (a decoded bitmap or an SVG document) stretched to
    /// fill `rect`.
    ///
    /// Painted for replaced elements such as `<img>` and inline `<svg>`. The
    /// renderer looks `image` up in its image store and draws nothing when
    /// it has no picture for it.
    Image {
        /// The replaced box's content box.
        rect: Rect,
        /// Which picture to draw.
        image: ImageKey,
    },
}

/// An ordered list of paint commands, drawn back to front.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DisplayList {
    /// Commands in draw order.
    pub items: Vec<DisplayItem>,
}

/// Converts standalone boxes into a display list, in the order given.
///
/// Each box with a background produces one [`DisplayItem::FillRect`]; boxes
/// without one produce nothing. For documents, use [`crate::paint_document`],
/// which also orders painting per CSS.
pub fn build_display_list(boxes: &[PaintBox]) -> DisplayList {
    let list = DisplayList {
        items: boxes
            .iter()
            .filter_map(|paint_box| {
                paint_box.background.map(|color| DisplayItem::FillRect {
                    rect: paint_box.rect,
                    color,
                })
            })
            .collect(),
    };
    log::debug!(
        "display list built from {} paint boxes: {} items",
        boxes.len(),
        list.items.len()
    );
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_boxes_yields_empty_display_list() {
        assert!(build_display_list(&[]).items.is_empty());
    }
}
