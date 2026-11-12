//! Typed CSS values from CSS Values and Units Level 4 and CSS Fonts Level 4.
//!
//! The cascade stores declarations as strings. These helpers parse the length
//! forms that layout consumes and compute `font-size` to the absolute value
//! that inheritance must pass to descendants.

/// The initial `font-size` (`medium`) in CSS pixels.
pub const MEDIUM_FONT_SIZE: f32 = 16.0;

/// A unit for a CSS `<length>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    /// CSS pixels (`px`).
    Px,
    /// The element's computed font size (`em`).
    Em,
    /// The root element's computed font size (`rem`).
    Rem,
    /// The font's x-height (`ex`), approximated as `0.5em`.
    Ex,
    /// The advance of the font's `0` glyph (`ch`), approximated as `0.5em`.
    Ch,
    /// Inches (`in`), 96 CSS pixels.
    In,
    /// Centimeters (`cm`), 96/2.54 CSS pixels.
    Cm,
    /// Millimeters (`mm`), 1/10 of a centimeter.
    Mm,
    /// Quarter-millimeters (`Q`), 1/40 of a centimeter.
    Q,
    /// Points (`pt`), 1/72 of an inch.
    Pt,
    /// Picas (`pc`), 12 points.
    Pc,
    /// 1% of the viewport width (`vw`).
    Vw,
    /// 1% of the viewport height (`vh`).
    Vh,
    /// 1% of the smaller viewport dimension (`vmin`).
    Vmin,
    /// 1% of the larger viewport dimension (`vmax`).
    Vmax,
}

/// A CSS `<length>`: a number with a unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// The numeric part, which may be negative.
    pub value: f32,
    /// The unit the number is measured in.
    pub unit: LengthUnit,
}

/// The context needed to convert relative lengths to CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LengthContext {
    /// The computed font size in CSS pixels used for `em`, `ex`, and `ch`.
    pub font_size: f32,
    /// The root element's computed font size in CSS pixels used for `rem`.
    pub root_font_size: f32,
    /// The viewport width in CSS pixels used for `vw`, `vmin`, and `vmax`.
    pub viewport_width: f32,
    /// The viewport height in CSS pixels used for `vh`, `vmin`, and `vmax`.
    pub viewport_height: f32,
}

impl Length {
    /// Converts the length to CSS pixels.
    ///
    /// `ex` and `ch` use the `0.5em` fallback that CSS Values 4 §6.1.1
    /// prescribes when glyph measurements are unavailable.
    pub fn to_px(self, context: &LengthContext) -> f32 {
        let cm = 96.0 / 2.54;
        let scale = match self.unit {
            LengthUnit::Px => 1.0,
            LengthUnit::Em => context.font_size,
            LengthUnit::Rem => context.root_font_size,
            LengthUnit::Ex | LengthUnit::Ch => context.font_size * 0.5,
            LengthUnit::In => 96.0,
            LengthUnit::Cm => cm,
            LengthUnit::Mm => cm / 10.0,
            LengthUnit::Q => cm / 40.0,
            LengthUnit::Pt => 96.0 / 72.0,
            LengthUnit::Pc => 16.0,
            LengthUnit::Vw => context.viewport_width / 100.0,
            LengthUnit::Vh => context.viewport_height / 100.0,
            LengthUnit::Vmin => context.viewport_width.min(context.viewport_height) / 100.0,
            LengthUnit::Vmax => context.viewport_width.max(context.viewport_height) / 100.0,
        };
        self.value * scale
    }

    /// Whether the length depends on the viewport size.
    pub fn is_viewport_relative(self) -> bool {
        matches!(
            self.unit,
            LengthUnit::Vw | LengthUnit::Vh | LengthUnit::Vmin | LengthUnit::Vmax
        )
    }
}

/// Parses a CSS `<number>` such as `1`, `-0.5`, `.25`, or `1e2`.
pub fn parse_number(input: &str) -> Option<f32> {
    let input = input.trim();
    let valid = !input.is_empty()
        && input
            .chars()
            .all(|character| character.is_ascii_digit() || "+-.eE".contains(character))
        && input.chars().any(|character| character.is_ascii_digit());
    if !valid {
        return None;
    }
    input.parse::<f32>().ok().filter(|value| value.is_finite())
}

/// Parses a CSS `<length>`.
///
/// Unit names are ASCII case-insensitive. A unitless `0` is accepted as
/// `0px`, as CSS allows. `calc()` and other math functions are not supported
/// and return `None`.
pub fn parse_length(input: &str) -> Option<Length> {
    let input = input.trim();
    let (number, unit) = input.split_at(number_prefix_len(input));
    let value = parse_number(number)?;
    let unit = match unit.to_ascii_lowercase().as_str() {
        "" if value == 0.0 => LengthUnit::Px,
        "px" => LengthUnit::Px,
        "em" => LengthUnit::Em,
        "rem" => LengthUnit::Rem,
        "ex" => LengthUnit::Ex,
        "ch" => LengthUnit::Ch,
        "in" => LengthUnit::In,
        "cm" => LengthUnit::Cm,
        "mm" => LengthUnit::Mm,
        "q" => LengthUnit::Q,
        "pt" => LengthUnit::Pt,
        "pc" => LengthUnit::Pc,
        "vw" => LengthUnit::Vw,
        "vh" => LengthUnit::Vh,
        "vmin" => LengthUnit::Vmin,
        "vmax" => LengthUnit::Vmax,
        _ => return None,
    };
    Some(Length { value, unit })
}

/// Returns the byte length of the CSS `<number>` token at the start of `input`.
///
/// An exponent is consumed only when digits follow it, so `1em` splits into
/// `1` and `em` while `1e2px` splits into `1e2` and `px`.
fn number_prefix_len(input: &str) -> usize {
    let bytes = input.as_bytes();
    let digits = |mut index: usize| {
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        index
    };
    let mut index = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    index = digits(index);
    if bytes.get(index) == Some(&b'.') && bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
        index = digits(index + 1);
    }
    if matches!(bytes.get(index), Some(b'e' | b'E')) {
        let mut exponent = index + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        if bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            index = digits(exponent);
        }
    }
    index
}

/// Parses a CSS `<percentage>` such as `50%` and returns the number before `%`.
pub fn parse_percentage(input: &str) -> Option<f32> {
    parse_number(input.trim().strip_suffix('%')?)
}

/// Computes a `font-size` declaration to an absolute size in CSS pixels.
///
/// `parent_font_size` resolves `em`, `ex`, `ch`, percentages, and the
/// `larger`/`smaller` keywords; `root_font_size` resolves `rem`. Absolute-size
/// keywords use the CSS Fonts 4 §2.5 table for a 16px `medium`. Negative
/// sizes are invalid. Viewport units return `None` because the cascade runs
/// without a viewport.
pub fn compute_font_size(value: &str, parent_font_size: f32, root_font_size: f32) -> Option<f32> {
    let value = value.trim();
    let keyword = match value.to_ascii_lowercase().as_str() {
        "xx-small" => Some(MEDIUM_FONT_SIZE * 3.0 / 5.0),
        "x-small" => Some(MEDIUM_FONT_SIZE * 3.0 / 4.0),
        "small" => Some(MEDIUM_FONT_SIZE * 8.0 / 9.0),
        "medium" => Some(MEDIUM_FONT_SIZE),
        "large" => Some(MEDIUM_FONT_SIZE * 6.0 / 5.0),
        "x-large" => Some(MEDIUM_FONT_SIZE * 3.0 / 2.0),
        "xx-large" => Some(MEDIUM_FONT_SIZE * 2.0),
        "xxx-large" => Some(MEDIUM_FONT_SIZE * 3.0),
        "larger" => Some(parent_font_size * 1.2),
        "smaller" => Some(parent_font_size / 1.2),
        _ => None,
    };
    if keyword.is_some() {
        return keyword;
    }
    let size = if let Some(percent) = parse_percentage(value) {
        parent_font_size * percent / 100.0
    } else {
        let length = parse_length(value)?;
        if length.is_viewport_relative() {
            return None;
        }
        length.to_px(&LengthContext {
            font_size: parent_font_size,
            root_font_size,
            viewport_width: 0.0,
            viewport_height: 0.0,
        })
    };
    (size >= 0.0).then_some(size)
}

/// Formats a CSS-pixel length as CSS text, for example `21.44px`.
pub fn format_px(value: f32) -> String {
    format!("{}px", format_number(value))
}

fn format_number(value: f32) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    format!("{rounded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lengths_with_units_and_unitless_zero() {
        assert_eq!(
            parse_length("1.5EM"),
            Some(Length {
                value: 1.5,
                unit: LengthUnit::Em
            })
        );
        assert_eq!(
            parse_length("-8px"),
            Some(Length {
                value: -8.0,
                unit: LengthUnit::Px
            })
        );
        assert_eq!(
            parse_length("0").map(|length| length.unit),
            Some(LengthUnit::Px)
        );
        assert_eq!(parse_length("5"), None);
        assert_eq!(parse_length("calc(1px + 2px)"), None);
        assert_eq!(parse_length("1e1px").map(|length| length.value), Some(10.0));
    }

    #[test]
    fn absolute_units_convert_to_css_pixels() {
        let context = LengthContext {
            font_size: 20.0,
            root_font_size: 10.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
        };
        let px = |input| parse_length(input).unwrap().to_px(&context);
        assert_eq!(px("1in"), 96.0);
        assert_eq!(px("12pt"), 16.0);
        assert_eq!(px("2em"), 40.0);
        assert_eq!(px("2rem"), 20.0);
        assert_eq!(px("10vw"), 80.0);
        assert_eq!(px("10vmin"), 60.0);
    }

    #[test]
    fn font_size_computes_against_the_parent() {
        assert_eq!(compute_font_size("2em", 16.0, 16.0), Some(32.0));
        assert_eq!(compute_font_size("150%", 20.0, 16.0), Some(30.0));
        assert_eq!(compute_font_size("x-large", 99.0, 16.0), Some(24.0));
        assert_eq!(compute_font_size("2rem", 99.0, 10.0), Some(20.0));
        assert_eq!(compute_font_size("-1px", 16.0, 16.0), None);
        assert_eq!(compute_font_size("10vw", 16.0, 16.0), None);
    }
}
