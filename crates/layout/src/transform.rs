//! CSS Transforms 1: the `transform` and `transform-origin` properties.
//!
//! Transforms are parsed into a list of 2D functions at style time and
//! resolved to an [`Affine`] matrix against a box's border box at paint
//! time, so percentages follow the box after layout moves it. They never
//! affect layout.
//!
//! Limitations: 3D functions are flattened with an orthographic
//! projection (`rotateX(a)` scales the y axis by `cos a`, `translateZ` and
//! `perspective` are ignored, and `matrix3d` keeps its 2D part), and
//! `transform-box` is always the border box.

use std::sync::Arc;

use css::values::{LengthContext, parse_length, parse_percentage};

use crate::{LengthPercentage, Rect};

/// A 2D affine matrix in CSS pixels, mapping `(x, y)` to
/// `(a * x + c * y + e, b * x + d * y + f)`, as CSS's `matrix(a, b, c, d, e, f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Affine {
    /// Horizontal scale component.
    pub a: f32,
    /// Vertical shear component.
    pub b: f32,
    /// Horizontal shear component.
    pub c: f32,
    /// Vertical scale component.
    pub d: f32,
    /// Horizontal translation in CSS pixels.
    pub e: f32,
    /// Vertical translation in CSS pixels.
    pub f: f32,
}

impl Affine {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A translation by `(x, y)` CSS pixels.
    pub fn translate(x: f32, y: f32) -> Self {
        Self {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }

    /// The matrix that applies `other` first and then `self`.
    pub fn then_after(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    /// Maps the point `(x, y)`.
    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The inverse matrix, or `None` when the matrix is singular (for
    /// example `scale(0)`), which makes the transformed content invisible.
    pub fn inverse(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        if !determinant.is_finite() || determinant.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (
            self.d / determinant,
            -self.b / determinant,
            -self.c / determinant,
            self.a / determinant,
        );
        Some(Self {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }

    /// The bounding box of `rect` after mapping it.
    pub fn bounds(self, rect: Rect) -> Rect {
        let corners = [
            self.apply(rect.x, rect.y),
            self.apply(rect.right(), rect.y),
            self.apply(rect.x, rect.bottom()),
            self.apply(rect.right(), rect.bottom()),
        ];
        let (mut left, mut top) = corners[0];
        let (mut right, mut bottom) = corners[0];
        for (x, y) in corners {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        Rect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }

    /// Whether this is the identity matrix.
    pub fn is_identity(self) -> bool {
        self == Self::IDENTITY
    }
}

/// One function of a `transform` list. Angles are in radians; lengths in
/// CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransformFunction {
    /// `matrix()`, or the 2D part of `matrix3d()`.
    Matrix(Affine),
    /// `translate()`, `translateX()`, `translateY()`, `translate3d()`;
    /// percentages resolve against the border box's width and height.
    Translate(LengthPercentage, LengthPercentage),
    /// `scale()`, `scaleX()`, `scaleY()`, `scale3d()`, and the flattened
    /// `rotateX()` / `rotateY()`.
    Scale(f32, f32),
    /// `rotate()` and `rotateZ()`, clockwise.
    Rotate(f32),
    /// `skew()`, `skewX()`, `skewY()`.
    Skew(f32, f32),
}

impl TransformFunction {
    /// The function's matrix for a reference box `width` by `height`.
    fn matrix(self, width: f32, height: f32) -> Affine {
        match self {
            Self::Matrix(matrix) => matrix,
            Self::Translate(x, y) => Affine::translate(x.resolve(width), y.resolve(height)),
            Self::Scale(x, y) => Affine {
                a: x,
                d: y,
                ..Affine::IDENTITY
            },
            Self::Rotate(angle) => {
                let (sin, cos) = angle.sin_cos();
                Affine {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    ..Affine::IDENTITY
                }
            }
            Self::Skew(x, y) => Affine {
                b: y.tan(),
                c: x.tan(),
                ..Affine::IDENTITY
            },
        }
    }
}

/// A computed `transform` with its `transform-origin`.
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    /// The functions, applied to the content from last to first (so the
    /// first function is outermost), as CSS specifies.
    pub functions: Arc<[TransformFunction]>,
    /// The `transform-origin` x and y, relative to the border box's top-left
    /// corner; percentages resolve against its width and height.
    pub origin: (LengthPercentage, LengthPercentage),
}

impl Transform {
    /// The matrix mapping content of a box with border box `rect` (absolute
    /// CSS pixels) to where it is drawn, origin included.
    pub fn matrix(&self, rect: Rect) -> Affine {
        let origin_x = rect.x + self.origin.0.resolve(rect.width);
        let origin_y = rect.y + self.origin.1.resolve(rect.height);
        let functions = self
            .functions
            .iter()
            .fold(Affine::IDENTITY, |matrix, function| {
                matrix.then_after(function.matrix(rect.width, rect.height))
            });
        Affine::translate(origin_x, origin_y)
            .then_after(functions)
            .then_after(Affine::translate(-origin_x, -origin_y))
    }
}

/// Parses an angle (`deg`, `rad`, `grad`, `turn`, or a unitless zero) to
/// radians.
fn parse_angle(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    let (number, factor) = if let Some(number) = value.strip_suffix("deg") {
        (number, std::f32::consts::PI / 180.0)
    } else if let Some(number) = value.strip_suffix("grad") {
        (number, std::f32::consts::PI / 200.0)
    } else if let Some(number) = value.strip_suffix("rad") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("turn") {
        (number, std::f32::consts::TAU)
    } else {
        return (value.parse::<f32>().ok() == Some(0.0)).then_some(0.0);
    };
    let number = number.parse::<f32>().ok().filter(|n| n.is_finite())?;
    Some(number * factor)
}

/// Parses a length or percentage, resolving lengths with `context`.
fn parse_length_percentage(value: &str, context: &LengthContext) -> Option<LengthPercentage> {
    if let Some(percent) = parse_percentage(value) {
        return Some(LengthPercentage::Percent(percent));
    }
    let px = parse_length(value)?.to_px(context);
    px.is_finite().then_some(LengthPercentage::Px(px))
}

/// Parses a number or a percentage (`50%` is `0.5`), as `scale()` takes.
fn parse_factor(value: &str) -> Option<f32> {
    let value = value.trim();
    parse_percentage(value)
        .map(|percent| percent / 100.0)
        .or_else(|| value.parse::<f32>().ok())
        .filter(|factor| factor.is_finite())
}

/// Parses a `transform` value. `none` gives `Some(empty list)`; invalid
/// values give `None` so the declaration is ignored.
pub(crate) fn parse_transform(
    value: &str,
    context: &LengthContext,
) -> Option<Vec<TransformFunction>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let mut functions = Vec::new();
    let mut rest = value;
    while !rest.trim().is_empty() {
        let trimmed = rest.trim_start();
        let open = trimmed.find('(')?;
        let close = open + trimmed[open..].find(')')?;
        let name = trimmed[..open].trim().to_ascii_lowercase();
        let args: Vec<&str> = trimmed[open + 1..close].split(',').map(str::trim).collect();
        // Every argument list is non-empty: `rotate()` has one empty argument.
        if args.iter().any(|arg| arg.is_empty()) {
            return None;
        }
        let length = |arg: &str| parse_length_percentage(arg, context);
        let zero = LengthPercentage::Px(0.0);
        let function = match (name.as_str(), args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => {
                let n = |v: &str| v.parse::<f32>().ok().filter(|n| n.is_finite());
                TransformFunction::Matrix(Affine {
                    a: n(a)?,
                    b: n(b)?,
                    c: n(c)?,
                    d: n(d)?,
                    e: n(e)?,
                    f: n(f)?,
                })
            }
            ("matrix3d", values) if values.len() == 16 => {
                let n = |i: usize| values[i].parse::<f32>().ok().filter(|n| n.is_finite());
                TransformFunction::Matrix(Affine {
                    a: n(0)?,
                    b: n(1)?,
                    c: n(4)?,
                    d: n(5)?,
                    e: n(12)?,
                    f: n(13)?,
                })
            }
            ("translate", [x]) => TransformFunction::Translate(length(x)?, zero),
            ("translate", [x, y]) => TransformFunction::Translate(length(x)?, length(y)?),
            ("translate3d", [x, y, z]) => {
                parse_length(z)?;
                TransformFunction::Translate(length(x)?, length(y)?)
            }
            ("translatex", [x]) => TransformFunction::Translate(length(x)?, zero),
            ("translatey", [y]) => TransformFunction::Translate(zero, length(y)?),
            ("translatez", [z]) => {
                parse_length(z)?;
                TransformFunction::Translate(zero, zero)
            }
            ("scale", [s]) => {
                let s = parse_factor(s)?;
                TransformFunction::Scale(s, s)
            }
            ("scale", [x, y]) => TransformFunction::Scale(parse_factor(x)?, parse_factor(y)?),
            ("scale3d", [x, y, z]) => {
                parse_factor(z)?;
                TransformFunction::Scale(parse_factor(x)?, parse_factor(y)?)
            }
            ("scalex", [x]) => TransformFunction::Scale(parse_factor(x)?, 1.0),
            ("scaley", [y]) => TransformFunction::Scale(1.0, parse_factor(y)?),
            ("scalez", [z]) => {
                parse_factor(z)?;
                TransformFunction::Scale(1.0, 1.0)
            }
            ("rotate" | "rotatez", [angle]) => TransformFunction::Rotate(parse_angle(angle)?),
            ("rotatex", [angle]) => TransformFunction::Scale(1.0, parse_angle(angle)?.cos()),
            ("rotatey", [angle]) => TransformFunction::Scale(parse_angle(angle)?.cos(), 1.0),
            ("skew", [x]) => TransformFunction::Skew(parse_angle(x)?, 0.0),
            ("skew", [x, y]) => TransformFunction::Skew(parse_angle(x)?, parse_angle(y)?),
            ("skewx", [x]) => TransformFunction::Skew(parse_angle(x)?, 0.0),
            ("skewy", [y]) => TransformFunction::Skew(0.0, parse_angle(y)?),
            ("perspective", [_]) => TransformFunction::Scale(1.0, 1.0),
            _ => {
                log::debug!("unsupported transform function {name}({})", args.join(", "));
                return None;
            }
        };
        functions.push(function);
        rest = &trimmed[close + 1..];
    }
    Some(functions)
}

/// Parses a `transform-origin` value: one or two positions (keywords,
/// lengths, or percentages) and an ignored z length. `None` for invalid
/// values. The initial value is `50% 50%`.
pub(crate) fn parse_transform_origin(
    value: &str,
    context: &LengthContext,
) -> Option<(LengthPercentage, LengthPercentage)> {
    /// One component of the position.
    #[derive(Clone, Copy)]
    enum Part {
        /// `left` or `right`: horizontal only.
        X(LengthPercentage),
        /// `top` or `bottom`: vertical only.
        Y(LengthPercentage),
        /// `center`: either axis.
        Center,
        /// A length or percentage: either axis, by position.
        Value(LengthPercentage),
    }
    let percent = LengthPercentage::Percent;
    let part = |text: &str| {
        Some(match text.to_ascii_lowercase().as_str() {
            "left" => Part::X(percent(0.0)),
            "right" => Part::X(percent(100.0)),
            "top" => Part::Y(percent(0.0)),
            "bottom" => Part::Y(percent(100.0)),
            "center" => Part::Center,
            _ => Part::Value(parse_length_percentage(text, context)?),
        })
    };
    let center = percent(50.0);
    let parts: Vec<&str> = value.split_ascii_whitespace().collect();
    match parts.as_slice() {
        [one] => Some(match part(one)? {
            Part::X(x) | Part::Value(x) => (x, center),
            Part::Y(y) => (center, y),
            Part::Center => (center, center),
        }),
        [first, second] | [first, second, _] => {
            let (mut first, mut second) = (part(first)?, part(second)?);
            // Keywords may come in either order: `top left` is `left top`.
            if matches!(first, Part::Y(_)) || matches!(second, Part::X(_)) {
                if matches!(first, Part::Value(_)) || matches!(second, Part::Value(_)) {
                    return None;
                }
                std::mem::swap(&mut first, &mut second);
            }
            let x = match first {
                Part::X(x) | Part::Value(x) => x,
                Part::Center => center,
                Part::Y(_) => return None,
            };
            let y = match second {
                Part::Y(y) | Part::Value(y) => y,
                Part::Center => center,
                Part::X(_) => return None,
            };
            Some((x, y))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> LengthContext {
        LengthContext {
            font_size: 10.0,
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
        }
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4
    }

    fn transform(value: &str, origin: &str) -> Transform {
        Transform {
            functions: parse_transform(value, &context()).unwrap().into(),
            origin: parse_transform_origin(origin, &context()).unwrap(),
        }
    }

    const BOX: Rect = Rect {
        x: 100.0,
        y: 50.0,
        width: 40.0,
        height: 20.0,
    };

    #[test]
    fn translate_resolves_percentages_against_the_border_box() {
        let matrix = transform("translate(50%, 1em)", "50% 50%").matrix(BOX);
        assert!(close(matrix.apply(100.0, 50.0), (120.0, 60.0)));
    }

    #[test]
    fn rotation_and_scale_pivot_on_the_origin() {
        let matrix = transform("rotate(90deg)", "center").matrix(BOX);
        // The center (120, 60) stays put; the top-left corner swings round.
        assert!(close(matrix.apply(120.0, 60.0), (120.0, 60.0)));
        assert!(close(matrix.apply(100.0, 50.0), (130.0, 40.0)));
        let scaled = transform("scale(2)", "left top").matrix(BOX);
        assert!(close(scaled.apply(140.0, 70.0), (180.0, 90.0)));
    }

    #[test]
    fn functions_apply_right_to_left() {
        let matrix = transform("translateX(10px) scale(2)", "0 0").matrix(BOX);
        // scale first (about 100,50), then translate.
        assert!(close(matrix.apply(110.0, 50.0), (130.0, 50.0)));
    }

    #[test]
    fn matrix_inverse_round_trips_and_singular_matrices_have_none() {
        let matrix = transform("rotate(30deg) skewX(10deg) scale(1.5, 0.5)", "center").matrix(BOX);
        let inverse = matrix.inverse().unwrap();
        let (x, y) = matrix.apply(107.0, 61.0);
        assert!(close(inverse.apply(x, y), (107.0, 61.0)));
        assert!(
            transform("scale(0)", "center")
                .matrix(BOX)
                .inverse()
                .is_none()
        );
    }

    #[test]
    fn transform_values_parse_and_invalid_ones_are_rejected() {
        let parse = |value| parse_transform(value, &context());
        assert_eq!(parse("none"), Some(Vec::new()));
        assert_eq!(
            parse("rotate(0.5turn)"),
            Some(vec![TransformFunction::Rotate(std::f32::consts::PI)])
        );
        assert_eq!(
            parse("translate3d(1px, 2px, 3px) scaleY(50%)"),
            Some(vec![
                TransformFunction::Translate(LengthPercentage::Px(1.0), LengthPercentage::Px(2.0)),
                TransformFunction::Scale(1.0, 0.5),
            ])
        );
        assert_eq!(parse("rotate(45)"), None);
        assert_eq!(parse("translate()"), None);
        assert_eq!(parse("wobble(2)"), None);
        assert_eq!(parse("scale(2"), None);
    }

    #[test]
    fn transform_origin_accepts_keywords_in_either_order() {
        let origin = |value| parse_transform_origin(value, &context());
        let p = LengthPercentage::Percent;
        assert_eq!(origin("left"), Some((p(0.0), p(50.0))));
        assert_eq!(origin("top"), Some((p(50.0), p(0.0))));
        assert_eq!(origin("top right"), Some((p(100.0), p(0.0))));
        assert_eq!(origin("right bottom"), Some((p(100.0), p(100.0))));
        assert_eq!(
            origin("10px 25%"),
            Some((LengthPercentage::Px(10.0), p(25.0)))
        );
        assert_eq!(origin("top bottom"), None);
    }
}
