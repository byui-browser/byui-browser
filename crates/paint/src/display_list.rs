//! Display-list commands.

use crate::{Color, PaintBox, Rect};

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
    /// Draws a run of text.
    Text {
        /// Left edge of the first glyph's advance.
        x: f32,
        /// The alphabetic baseline the glyphs sit on.
        baseline: f32,
        /// Font size; the renderer scales its glyphs to it.
        font_size: f32,
        /// Text to draw.
        text: String,
        /// Text color.
        color: Color,
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
    DisplayList {
        items: boxes
            .iter()
            .filter_map(|paint_box| {
                paint_box.background.map(|color| DisplayItem::FillRect {
                    rect: paint_box.rect,
                    color,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_boxes_yields_empty_display_list() {
        assert!(build_display_list(&[]).items.is_empty());
    }
}
