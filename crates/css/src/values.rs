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
/// `0px`, as CSS allows. Complex `calc()` expressions are left for the layout
/// value resolver and are not converted by this helper.
pub fn parse_length(input: &str) -> Option<Length> {
    let input = input.trim();
    if input.len() >= 6 && input[..5].eq_ignore_ascii_case("calc(") && input.ends_with(')') {
        return None;
    }
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

#[allow(dead_code)]
fn parse_calc_length(input: &str) -> Option<Length> {
    let mut expression = input.trim();
    let mut result: Option<Length> = None;
    let mut operation = 1.0_f32;
    while !expression.is_empty() {
        let split = expression
            .char_indices()
            .skip(1)
            .find(|(_, c)| matches!(c, '+' | '-'))
            .map(|(i, _)| i);
        let (term_text, rest) = split
            .map(|i| expression.split_at(i))
            .unwrap_or((expression, ""));
        let term = parse_length(term_text.trim())?;
        result = Some(match result {
            Some(current) if current.unit == term.unit => Length {
                value: current.value + operation * term.value,
                unit: current.unit,
            },
            Some(_) => return None,
            None => Length {
                value: operation * term.value,
                unit: term.unit,
            },
        });
        expression = rest.trim_start();
        if expression.is_empty() {
            break;
        }
        let operator = expression.chars().next()?;
        operation = match operator {
            '+' => 1.0,
            '-' => -1.0,
            _ => return None,
        };
        expression = expression[operator.len_utf8()..].trim_start();
    }
    result
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
/// sizes are invalid. The legacy entry point intentionally has no viewport and
/// therefore returns `None` for viewport units.
pub fn compute_font_size(value: &str, parent_font_size: f32, root_font_size: f32) -> Option<f32> {
    compute_font_size_in_context(value, parent_font_size, root_font_size, 0.0, 0.0, false)
}

/// Computes a `font-size` declaration with viewport units enabled.
pub fn compute_font_size_with_context(
    value: &str,
    parent_font_size: f32,
    root_font_size: f32,
    viewport_width: f32,
    viewport_height: f32,
) -> Option<f32> {
    compute_font_size_in_context(
        value,
        parent_font_size,
        root_font_size,
        viewport_width,
        viewport_height,
        true,
    )
}

fn compute_font_size_in_context(
    value: &str,
    parent_font_size: f32,
    root_font_size: f32,
    viewport_width: f32,
    viewport_height: f32,
    allow_viewport: bool,
) -> Option<f32> {
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
        if length.is_viewport_relative() && !allow_viewport {
            return None;
        }
        length.to_px(&LengthContext {
            font_size: parent_font_size,
            root_font_size,
            viewport_width,
            viewport_height,
        })
    };
    (size >= 0.0).then_some(size)
}


/// Resolves CSS length functions whose arguments use one compatible unit.
///
/// This covers the length forms the layout engine can consume directly:
/// `calc()`, `min()`, `max()`, and `clamp()`. Mixed-unit expressions are
/// left unresolved for a future layout context.
pub fn resolve_length_function(input: &str) -> Option<String> {
    let input = input.trim();
    for name in ["calc", "min", "max", "clamp"] {
        let Some(inner) = function_inner(input, name) else {
            continue;
        };
        if name == "calc" {
            return calculate_length(inner).map(format_length);
        }
        let args = split_function_args(inner);
        if (name == "clamp" && args.len() != 3)
            || (name != "clamp" && args.len() < 2)
        {
            return None;
        }
        let lengths: Vec<_> = args
            .iter()
            .map(|argument| parse_length(argument.trim()))
            .collect::<Option<Vec<_>>>()?;
        let unit = lengths[0].unit;
        if lengths.iter().any(|length| length.unit != unit) {
            return None;
        }
        let selected = match name {
            "min" => lengths
                .iter()
                .min_by(|left, right| left.value.total_cmp(&right.value))?,
            "max" => lengths
                .iter()
                .max_by(|left, right| left.value.total_cmp(&right.value))?,
            "clamp" => {
                let minimum = lengths[0].value;
                let preferred = lengths[1].value;
                let maximum = lengths[2].value;
                &Length {
                    value: preferred.clamp(minimum, maximum),
                    unit,
                }
            }
            _ => unreachable!(),
        };
        return Some(format_length(*selected));
    }
    None
}

fn function_inner<'a>(input: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}(");
    if input.len() <= prefix.len()
        || !input[..prefix.len()].eq_ignore_ascii_case(&prefix)
        || !input.ends_with(')')
    {
        return None;
    }
    Some(&input[prefix.len()..input.len() - 1])
}

fn split_function_args(input: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in input.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(input[start..index].trim());
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    args.push(input[start..].trim());
    args
}

fn calculate_length(input: &str) -> Option<Length> {
    let mut expression = input.trim();
    let mut result = None;
    let mut operation = 1.0f32;
    while !expression.is_empty() {
        let split = expression
            .char_indices()
            .skip(1)
            .find(|(_, character)| matches!(character, '+' | '-'))
            .map(|(index, _)| index);
        let (term, rest) = split
            .map(|index| expression.split_at(index))
            .unwrap_or((expression, ""));
        let term = parse_length(term.trim())?;
        result = Some(match result {
            Some(current) if current.unit == term.unit => Length {
                value: current.value + operation * term.value,
                unit: current.unit,
            },
            Some(_) => return None,
            None => Length {
                value: operation * term.value,
                unit: term.unit,
            },
        });
        expression = rest.trim_start();
        if expression.is_empty() {
            break;
        }
        let operator = expression.chars().next()?;
        operation = match operator {
            '+' => 1.0,
            '-' => -1.0,
            _ => return None,
        };
        expression = expression[operator.len_utf8()..].trim_start();
    }
    result
}

fn format_length(length: Length) -> String {
    let unit = match length.unit {
        LengthUnit::Px => "px",
        LengthUnit::Em => "em",
        LengthUnit::Rem => "rem",
        LengthUnit::Ex => "ex",
        LengthUnit::Ch => "ch",
        LengthUnit::In => "in",
        LengthUnit::Cm => "cm",
        LengthUnit::Mm => "mm",
        LengthUnit::Q => "Q",
        LengthUnit::Pt => "pt",
        LengthUnit::Pc => "pc",
        LengthUnit::Vw => "vw",
        LengthUnit::Vh => "vh",
        LengthUnit::Vmin => "vmin",
        LengthUnit::Vmax => "vmax",
    };
    format!("{}{}", format_number(length.value), unit)
}

/// Formats a CSS-pixel length as CSS text, for example `21.44px`.
pub fn format_px(value: f32) -> String {
    format!("{}px", format_number(value))
}

fn format_number(value: f32) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    format!("{rounded}")
}
