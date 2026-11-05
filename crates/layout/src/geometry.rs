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
    /// The x coordinate of the right edge.
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// The y coordinate of the bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Returns this rectangle moved by `dx` and `dy` CSS pixels.
    pub fn translate(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..self
        }
    }

    /// Returns whether a CSS-pixel point is inside the rectangle.
    ///
    /// The top and left edges are inclusive; the bottom and right edges are
    /// exclusive.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.width && py < self.y + self.height
    }
}

/// One value per box side, in the CSS order top, right, bottom, left.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Sides<T> {
    /// The top side.
    pub top: T,
    /// The right side.
    pub right: T,
    /// The bottom side.
    pub bottom: T,
    /// The left side.
    pub left: T,
}

impl<T> Sides<T> {
    /// Creates sides from a `[top, right, bottom, left]` array.
    pub fn from_array([top, right, bottom, left]: [T; 4]) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Applies `f` to every side.
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Sides<U> {
        Sides {
            top: f(self.top),
            right: f(self.right),
            bottom: f(self.bottom),
            left: f(self.left),
        }
    }
}

/// Widths of a box's margin, border, or padding on each side, in CSS pixels.
pub type Edges = Sides<f32>;

impl Edges {
    /// The sum of the left and right widths.
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// The sum of the top and bottom widths.
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
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
