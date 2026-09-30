//! Display list generation from the layout tree.
//!
//! **Owning team**: Layout & Rendering Team
//!
//! Clear separation from GPU work: this crate produces paint commands /
//! display lists; `render` consumes them.

#![forbid(unsafe_code)]

/// RGBA color, 8 bits per channel.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Rectangle in device pixels.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Placeholder for the layout output handed over by `layout`.
///
/// Will be replaced by the real `LayoutTree` once its shape is fixed.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintBox {
    pub rect: Rect,
    pub background: Option<Color>,
}

/// One drawing command.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    FillRect { rect: Rect, color: Color },
}

/// Ordered drawing commands, back to front.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

/// Converts laid-out boxes into drawing commands.
///
/// Currently only handles an empty box list.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
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
