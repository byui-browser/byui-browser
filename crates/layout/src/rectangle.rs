//! Helpers for creating layout rectangles.

use crate::Rect;

/// Creates an axis-aligned rectangle from its top-left position and dimensions.
///
/// All values are measured in CSS pixels. The returned [`Rect`] is a plain
/// geometry value; it does not draw or paint anything by itself.
pub fn create_rectangle(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}
