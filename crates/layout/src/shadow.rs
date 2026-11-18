//! `box-shadow` values (CSS Backgrounds 3 §7.1).

use css::CssColor;
use css::values::{LengthContext, parse_length};

/// One shadow from a computed `box-shadow` list, with lengths in CSS pixels.
///
/// Outer shadows are painted below the box's background, offset from the
/// border box, grown by `spread`, and blurred by `blur`. Inset shadows are
/// parsed and kept here, but painting them is not supported yet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoxShadow {
    /// Horizontal offset; positive moves the shadow right.
    pub offset_x: f32,
    /// Vertical offset; positive moves the shadow down.
    pub offset_y: f32,
    /// Blur radius, never negative. The shadow edge fades over about this
    /// many pixels on each side (a Gaussian with standard deviation
    /// `blur / 2`).
    pub blur: f32,
    /// Spread distance; positive grows the shadow shape, negative shrinks it.
    pub spread: f32,
    /// Shadow color. An omitted color is the element's `color`.
    pub color: CssColor,
    /// Whether this is an `inset` shadow, drawn inside the padding box.
    pub inset: bool,
}

/// Parses a computed `box-shadow` value into its shadows, first (topmost)
/// shadow first.
///
/// `current_color` is used when a shadow omits its color or names
/// `currentcolor`. Returns an empty list for `none`, and `None` when any
/// shadow in the list is invalid, which makes the whole declaration invalid.
pub(crate) fn parse_box_shadow(
    value: &str,
    context: &LengthContext,
    current_color: CssColor,
) -> Option<Vec<BoxShadow>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Some(Vec::new());
    }
    let shadows = split_top_level(value, ',');
    if shadows.is_empty() {
        return None;
    }
    shadows
        .into_iter()
        .map(|shadow| parse_one(shadow, context, current_color))
        .collect()
}

/// Parses one comma-separated shadow:
/// `inset? && <length>{2,4} && <color>?`, in any order of the three groups.
fn parse_one(text: &str, context: &LengthContext, current_color: CssColor) -> Option<BoxShadow> {
    let mut inset = false;
    let mut lengths = Vec::new();
    let mut color = None;
    // The lengths must be adjacent; a color or `inset` ends the run.
    let mut lengths_done = false;
    for token in split_top_level(text, ' ') {
        if token.eq_ignore_ascii_case("inset") {
            if inset {
                return None;
            }
            inset = true;
            lengths_done |= !lengths.is_empty();
        } else if let Some(length) = parse_length(token) {
            if lengths_done || lengths.len() == 4 {
                return None;
            }
            lengths.push(length.to_px(context));
        } else {
            if color.is_some() {
                return None;
            }
            color = Some(if token.eq_ignore_ascii_case("currentcolor") {
                current_color
            } else if token.eq_ignore_ascii_case("transparent") {
                CssColor {
                    a: 0,
                    ..current_color
                }
            } else {
                css::parse_color(token)?
            });
            lengths_done |= !lengths.is_empty();
        }
    }
    let [offset_x, offset_y, blur, spread] = match lengths[..] {
        [x, y] => [x, y, 0.0, 0.0],
        [x, y, blur] => [x, y, blur, 0.0],
        [x, y, blur, spread] => [x, y, blur, spread],
        _ => return None,
    };
    if blur < 0.0 {
        return None;
    }
    Some(BoxShadow {
        offset_x,
        offset_y,
        blur,
        spread,
        color: color.unwrap_or(current_color),
        inset,
    })
}

/// Splits `text` on `separator` outside parentheses, dropping empty parts.
/// A space separator splits on any run of ASCII white space.
fn split_top_level(text: &str, separator: char) -> Vec<&str> {
    let is_separator = |character: char| {
        if separator == ' ' {
            character.is_ascii_whitespace()
        } else {
            character == separator
        }
    };
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && is_separator(character) => {
                parts.push(&text[start..index]);
                start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    fn parse(value: &str) -> Option<Vec<BoxShadow>> {
        let context = LengthContext {
            font_size: 10.0,
            root_font_size: 16.0,
            viewport_width: 800.0,
            viewport_height: 600.0,
        };
        parse_box_shadow(value, &context, BLACK)
    }

    #[test]
    fn parses_offsets_blur_spread_color_and_inset_in_any_order() {
        let shadows = parse("2px 3px 4px 1em red, inset 1px 1px rgb(0 0 255 / 50%)").unwrap();
        assert_eq!(shadows.len(), 2);
        assert_eq!(
            (
                shadows[0].offset_x,
                shadows[0].offset_y,
                shadows[0].blur,
                shadows[0].spread
            ),
            (2.0, 3.0, 4.0, 10.0)
        );
        assert_eq!(shadows[0].color, css::parse_color("red").unwrap());
        assert!(!shadows[0].inset);
        assert!(shadows[1].inset);
        assert_eq!(shadows[1].color.b, 255);
        assert_eq!(shadows[1].color.a, 128);
        let colored_first = parse("#00ff00 0 5px inset").unwrap();
        assert_eq!(
            (colored_first[0].offset_y, colored_first[0].inset),
            (5.0, true)
        );
        assert_eq!(colored_first[0].color.g, 255);
    }

    #[test]
    fn a_missing_color_is_the_current_color() {
        let shadows = parse("1px 1px").unwrap();
        assert_eq!(shadows[0].color, BLACK);
        assert_eq!(parse("1px 1px currentColor").unwrap()[0].color, BLACK);
    }

    #[test]
    fn none_is_empty_and_invalid_lists_are_rejected() {
        assert_eq!(parse("none"), Some(Vec::new()));
        for invalid in [
            "1px",
            "1px 2px 3px 4px 5px",
            "1px 2px -3px",
            "1px red 2px",
            "1px 1px red blue",
            "inset inset 1px 1px",
            "1px 1px, banana",
            "",
        ] {
            assert_eq!(parse(invalid), None, "{invalid}");
        }
    }
}
