//! Solid CSS colors used by computed style and painting.

/// An sRGB color with 8-bit red, green, blue, and alpha channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CssColor {
    /// Red channel, from 0 to 255.
    pub r: u8,
    /// Green channel, from 0 to 255.
    pub g: u8,
    /// Blue channel, from 0 to 255.
    pub b: u8,
    /// Alpha channel, from 0 (transparent) to 255 (opaque).
    pub a: u8,
}

/// Parses the solid color forms supported by the first renderer slice.
///
/// Supports `#rgb`, `#rrggbb`, common opaque named colors, and `rgb()` /
/// `rgba()` with integer channels from 0 to 255 and an optional alpha. Both
/// the legacy comma syntax (`rgba(1, 2, 3, 0.5)`) and the space syntax
/// (`rgb(1 2 3 / 50%)`) are accepted; alpha is a number from 0 to 1 or a
/// percentage. Other CSS color forms, including `transparent` and
/// `currentcolor`, return `None`.
pub fn parse_color(value: &str) -> Option<CssColor> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if !hex.is_ascii() {
            return None;
        }
        let (r, g, b) = match hex.len() {
            3 => {
                let mut digits = hex.chars();
                let channel = |digit: char| digit.to_digit(16).map(|value| (value * 17) as u8);
                (
                    channel(digits.next()?)?,
                    channel(digits.next()?)?,
                    channel(digits.next()?)?,
                )
            }
            6 => (
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ),
            _ => return None,
        };
        return Some(CssColor { r, g, b, a: 255 });
    }
    let lower = value.to_ascii_lowercase();
    if let Some(channels) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return parse_rgb_channels(channels);
    }
    let (r, g, b, a) = match lower.as_str() {
        "black" => (0, 0, 0, 255),
        "silver" => (192, 192, 192, 255),
        "gray" | "grey" => (128, 128, 128, 255),
        "white" => (255, 255, 255, 255),
        "maroon" => (128, 0, 0, 255),
        "red" => (255, 0, 0, 255),
        "purple" => (128, 0, 128, 255),
        "fuchsia" => (255, 0, 255, 255),
        "green" => (0, 128, 0, 255),
        "lime" => (0, 255, 0, 255),
        "olive" => (128, 128, 0, 255),
        "yellow" => (255, 255, 0, 255),
        "navy" => (0, 0, 128, 255),
        "blue" => (0, 0, 255, 255),
        "teal" => (0, 128, 128, 255),
        "aqua" => (0, 255, 255, 255),
        "orange" => (255, 165, 0, 255),
        "rebeccapurple" => (102, 51, 153, 255),
        _ => return None,
    };
    Some(CssColor { r, g, b, a })
}

/// Parses the inside of `rgb(...)` or `rgba(...)`.
fn parse_rgb_channels(channels: &str) -> Option<CssColor> {
    let (rgb, alpha) = if channels.contains(',') {
        let parts: Vec<_> = channels.split(',').map(str::trim).collect();
        match parts.as_slice() {
            [r, g, b] => ([*r, *g, *b], None),
            [r, g, b, a] => ([*r, *g, *b], Some(*a)),
            _ => return None,
        }
    } else {
        let (rgb, alpha) = match channels.split_once('/') {
            Some((rgb, alpha)) => (rgb, Some(alpha.trim())),
            None => (channels, None),
        };
        let parts: Vec<_> = rgb.split_ascii_whitespace().collect();
        let [r, g, b] = parts.as_slice() else {
            return None;
        };
        ([*r, *g, *b], alpha)
    };
    let alpha = match alpha {
        None => 255,
        Some(text) => {
            let fraction = match text.strip_suffix('%') {
                Some(percent) => percent.trim().parse::<f32>().ok()? / 100.0,
                None => text.parse::<f32>().ok()?,
            };
            if !fraction.is_finite() {
                return None;
            }
            (fraction.clamp(0.0, 1.0) * 255.0).round() as u8
        }
    };
    Some(CssColor {
        r: rgb[0].parse().ok()?,
        g: rgb[1].parse().ok()?,
        b: rgb[2].parse().ok()?,
        a: alpha,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_color_forms_and_rejects_unsupported_values() {
        assert_eq!(
            parse_color("red"),
            Some(CssColor {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            })
        );
        assert_eq!(
            parse_color("#abc"),
            Some(CssColor {
                r: 170,
                g: 187,
                b: 204,
                a: 255
            })
        );
        assert_eq!(
            parse_color("rgb(18, 52, 86)"),
            Some(CssColor {
                r: 18,
                g: 52,
                b: 86,
                a: 255
            })
        );
        assert_eq!(
            parse_color("rgba(18, 52, 86, 0.5)"),
            Some(CssColor {
                r: 18,
                g: 52,
                b: 86,
                a: 128
            })
        );
        assert_eq!(
            parse_color("rgb(1 2 3 / 0%)"),
            Some(CssColor {
                r: 1,
                g: 2,
                b: 3,
                a: 0
            })
        );
        assert_eq!(parse_color("rgb(1, 2)"), None);
        assert_eq!(parse_color("transparent"), None);
        assert_eq!(parse_color("#ééé"), None);
        assert_eq!(parse_color("rgb(256, 0, 0)"), None);
    }
}
