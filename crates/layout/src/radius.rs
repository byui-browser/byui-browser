//! Border radii (CSS Backgrounds and Borders 3 §5).

use crate::Rect;

/// A `<length-percentage>`: a length already resolved to CSS pixels, or a
/// percentage that resolves against a box dimension at paint time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentage {
    /// A length in CSS pixels.
    Px(f32),
    /// A percentage, where `50.0` means 50%.
    Percent(f32),
}

impl LengthPercentage {
    /// Resolves against `basis` CSS pixels.
    pub fn resolve(self, basis: f32) -> f32 {
        match self {
            Self::Px(px) => px,
            Self::Percent(percent) => basis * percent / 100.0,
        }
    }
}

impl Default for LengthPercentage {
    fn default() -> Self {
        Self::Px(0.0)
    }
}

/// The two radii of one elliptical corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CornerRadius<T> {
    /// The radius along the x axis.
    pub horizontal: T,
    /// The radius along the y axis.
    pub vertical: T,
}

/// One value per box corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Corners<T> {
    /// The top-left corner.
    pub top_left: T,
    /// The top-right corner.
    pub top_right: T,
    /// The bottom-right corner.
    pub bottom_right: T,
    /// The bottom-left corner.
    pub bottom_left: T,
}

impl<T> Corners<T> {
    /// Creates corners from a `[top-left, top-right, bottom-right,
    /// bottom-left]` array, the order `border-radius` lists them.
    pub fn from_array([top_left, top_right, bottom_right, bottom_left]: [T; 4]) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    /// Returns the corners as a `[top-left, top-right, bottom-right,
    /// bottom-left]` array.
    pub fn into_array(self) -> [T; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }
}

/// The computed `border-*-radius` values of a box. Percentages are kept and
/// resolve against the border box in [`BorderRadius::resolve`].
pub type BorderRadius = Corners<CornerRadius<LengthPercentage>>;

/// Used corner radii in CSS pixels, as painted.
pub type CornerRadii = Corners<CornerRadius<f32>>;

impl BorderRadius {
    /// Resolves the radii for a border box of `rect`'s size.
    ///
    /// Horizontal percentages resolve against the box width and vertical ones
    /// against its height. A corner with a zero radius on either axis is
    /// square. When adjacent radii would overlap, all radii shrink by the same
    /// factor until they fit (CSS Backgrounds 3 §5.5).
    pub fn resolve(&self, rect: Rect) -> CornerRadii {
        let (width, height) = (rect.width.max(0.0), rect.height.max(0.0));
        let mut radii = Corners::from_array(self.into_array().map(|corner| {
            let horizontal = corner.horizontal.resolve(width).max(0.0);
            let vertical = corner.vertical.resolve(height).max(0.0);
            if horizontal > 0.0 && vertical > 0.0 {
                CornerRadius {
                    horizontal,
                    vertical,
                }
            } else {
                CornerRadius::default()
            }
        }));
        let factor = |length: f32, a: f32, b: f32| {
            if a + b > 0.0 { length / (a + b) } else { 1.0 }
        };
        let scale = [
            factor(width, radii.top_left.horizontal, radii.top_right.horizontal),
            factor(
                width,
                radii.bottom_left.horizontal,
                radii.bottom_right.horizontal,
            ),
            factor(height, radii.top_left.vertical, radii.bottom_left.vertical),
            factor(
                height,
                radii.top_right.vertical,
                radii.bottom_right.vertical,
            ),
        ]
        .into_iter()
        .fold(1.0_f32, f32::min);
        if scale < 1.0 {
            radii = Corners::from_array(radii.into_array().map(|corner| CornerRadius {
                horizontal: corner.horizontal * scale,
                vertical: corner.vertical * scale,
            }));
        }
        radii
    }
}

impl CornerRadii {
    /// Whether every corner is square.
    pub fn is_zero(&self) -> bool {
        self.into_array()
            .iter()
            .all(|corner| corner.horizontal <= 0.0 || corner.vertical <= 0.0)
    }

    /// Returns every radius multiplied by `factor`, for example to convert
    /// CSS pixels to device pixels.
    pub fn scaled(self, factor: f32) -> Self {
        Corners::from_array(self.into_array().map(|corner| CornerRadius {
            horizontal: corner.horizontal * factor,
            vertical: corner.vertical * factor,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(value: LengthPercentage) -> BorderRadius {
        Corners::from_array(
            [CornerRadius {
                horizontal: value,
                vertical: value,
            }; 4],
        )
    }

    fn rect(width: f32, height: f32) -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    #[test]
    fn percentages_resolve_per_axis() {
        let radii = uniform(LengthPercentage::Percent(50.0)).resolve(rect(80.0, 30.0));
        assert_eq!(
            radii.top_left,
            CornerRadius {
                horizontal: 40.0,
                vertical: 15.0
            }
        );
    }

    #[test]
    fn overlapping_radii_shrink_proportionally() {
        let radii = uniform(LengthPercentage::Px(20.0)).resolve(rect(80.0, 30.0));
        // The 30px height fits 15px per corner, so every radius scales by 0.75.
        assert_eq!(
            radii.bottom_right,
            CornerRadius {
                horizontal: 15.0,
                vertical: 15.0
            }
        );
    }

    #[test]
    fn a_zero_component_makes_the_corner_square() {
        let mut radius = uniform(LengthPercentage::Px(4.0));
        radius.top_left.vertical = LengthPercentage::Px(0.0);
        let radii = radius.resolve(rect(80.0, 30.0));
        assert_eq!(radii.top_left, CornerRadius::default());
        assert!(!radii.is_zero());
        assert!(BorderRadius::default().resolve(rect(1.0, 1.0)).is_zero());
    }
}
