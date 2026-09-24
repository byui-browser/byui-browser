//! Display-list commands and the legacy conversion entry point.

use crate::{Color, PaintBox, Rect};

/// One ordered drawing command.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    /// Fills a CSS-pixel rectangle with an RGBA color.
    FillRect {
        /// Rectangle to fill.
        rect: Rect,
        /// Opaque or translucent fill color.
        color: Color,
    },
    /// Draws text at a CSS-pixel position using the renderer's bitmap font.
    Text {
        /// Horizontal position.
        x: f32,
        /// Vertical position.
        y: f32,
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

/// Converts legacy paint boxes into a display list.
///
/// The legacy non-empty path remains a placeholder while callers migrate to
/// [`crate::paint_document`]. Empty input returns an empty list.
pub fn build_display_list(boxes: &[PaintBox]) -> DisplayList {
    if boxes.is_empty() {
        return DisplayList::default();
    }
    todo!("TODO(paint): paint {} boxes", boxes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_boxes_yields_empty_display_list() {
        assert!(build_display_list(&[]).items.is_empty());
    }
}
