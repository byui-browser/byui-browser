//! Geometry primitives shared by layout stages.

/// A width and height measured in CSS pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

/// An axis-aligned rectangle measured in CSS pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    /// Horizontal position of the top-left corner.
    pub x: f32,
    /// Vertical position of the top-left corner.
    pub y: f32,
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

impl Rect {
    /// Returns whether a CSS-pixel point is inside the rectangle.
    ///
    /// The top and left edges are inclusive; the bottom and right edges are
    /// exclusive.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.width && py < self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_inclusive_top_left_exclusive_bottom_right() {
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            width: 5.0,
            height: 5.0,
        };
        assert!(rect.contains(10.0, 10.0));
        assert!(rect.contains(14.9, 14.9));
        assert!(!rect.contains(15.0, 15.0));
        assert!(!rect.contains(9.9, 12.0));
    }
}
