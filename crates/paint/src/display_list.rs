//! Display-list commands and legacy conversion entry point.

use crate::{Color, PaintBox, Rect};

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    FillRect {
        rect: Rect,
        color: Color,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        color: Color,
    },
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

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
