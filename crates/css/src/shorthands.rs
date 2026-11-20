//! Shorthand property expansion (CSS Cascading and Inheritance 4 §1.2).
//!
//! Shorthands are expanded to their longhands before the cascade so that, for
//! example, an author `margin: 0` competes with a user-agent `margin-top`.

use crate::Declaration;

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// The `border-*-radius` longhands, in the order `border-radius` lists them.
const CORNERS: [&str; 4] = ["top-left", "top-right", "bottom-right", "bottom-left"];

/// CSS-wide keywords, which a shorthand copies to each longhand unchanged.
const CSS_WIDE_KEYWORDS: [&str; 5] = ["inherit", "initial", "unset", "revert", "revert-layer"];

/// Expands one declaration into the longhand declarations it sets.
///
/// The box shorthands `margin` and `padding` are expanded using the
/// one-to-four value rule, as is `inset`, which sets the `top`, `right`,
/// `bottom`, and `left` longhands (CSS Positioned Layout 3 §3.1). `border-radius` is expanded to the four
/// `border-*-radius` longhands (CSS Backgrounds 3 §5.1): the values before an
/// optional `/` are the horizontal radii and the values after it the vertical
/// radii, each following the one-to-four value rule starting at the top-left
/// corner. A longhand gets `"<horizontal> <vertical>"`, or just the
/// horizontal radius when there is no `/`. Values are not validated here.
///
/// `animation` is expanded to the eight `animation-*` longhands, each a
/// comma-separated list with one entry per animation (CSS Animations 1
/// §3.10). The supported WebKit animation and flexbox aliases normalize to
/// their standard property names; legacy box alignment values are translated
/// to their modern flex equivalents.
/// `background` currently expands only its color component; image and other
/// layer components are ignored. Numeric `flex` forms and the common
/// `-webkit-flex`, `-webkit-box-flex`, `-webkit-box-sizing`, and
/// `-webkit-box-shadow` aliases map to the corresponding supported longhands.
/// `font` expands to `font-style`, `font-variant`, `font-weight`,
/// `font-stretch`, `font-size`, `line-height`, and `font-family`, resetting
/// omitted components to `normal` (CSS Fonts 4 §2.8). System font keywords
/// such as `caption` are not supported and drop the declaration. `overflow`
/// expands to `overflow-x` and `overflow-y`. The `-webkit-`, `-ms-`,
/// `-moz-`, and `-o-` prefixed `transform` and `transform-origin` map to the
/// standard properties.
/// `outline` expands width, style, and color. `text-decoration` currently
/// expands its line component; style, thickness, and explicit color are not
/// interpreted.
///
/// Other declarations are returned unchanged. An invalid shorthand value
/// expands to nothing, matching CSS's rule that invalid declarations are
/// dropped. CSS-wide keywords such as `inherit` are copied to every longhand.
pub fn expand(declaration: &Declaration) -> Vec<Declaration> {
    // `-ms-`, `-moz-`, and `-o-` only ever prefixed the transform properties
    // this engine supports; other prefixed properties pass through unchanged.
    for prefix in ["-ms-", "-moz-", "-o-"] {
        if let Some(property) = declaration.property.strip_prefix(prefix)
            && matches!(property, "transform" | "transform-origin")
        {
            return vec![Declaration {
                property: property.to_owned(),
                value: declaration.value.clone(),
            }];
        }
    }
    if let Some(property) = declaration.property.strip_prefix("-webkit-") {
        let (property, value) = match property {
            "box-align" => ("align-items", legacy_box_align(&declaration.value)),
            "box-pack" => ("justify-content", legacy_box_pack(&declaration.value)),
            "align-items" | "justify-content" | "animation" | "transform" | "transform-origin" => {
                (property, declaration.value.clone())
            }
            "box-shadow" => ("box-shadow", declaration.value.clone()),
            "box-sizing" => ("box-sizing", declaration.value.clone()),
            "box-flex" => ("flex-grow", declaration.value.clone()),
            "flex" => return expand_flex(&declaration.value),
            _ => return vec![declaration.clone()],
        };
        return expand(&Declaration {
            property: property.to_owned(),
            value,
        });
    }
    if declaration.property == "background" {
        return expand_background(&declaration.value);
    }
    if declaration.property == "outline" {
        return expand_outline(&declaration.value);
    }
    if declaration.property == "text-decoration" {
        return vec![Declaration {
            property: "text-decoration-line".to_owned(),
            value: declaration.value.clone(),
        }];
    }
    if declaration.property == "flex" {
        return expand_flex(&declaration.value);
    }
    if declaration.property == "font" {
        return expand_font(&declaration.value).unwrap_or_else(|| {
            log::debug!(
                "dropping invalid font shorthand value `{}`",
                declaration.value
            );
            Vec::new()
        });
    }
    if declaration.property == "overflow" {
        return expand_overflow(&declaration.value);
    }
    if matches!(
        declaration.property.as_str(),
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left"
    ) {
        return expand_border(declaration);
    }
    if matches!(
        declaration.property.as_str(),
        "border-color" | "border-width" | "border-style"
    ) {
        let parts = split_components(&declaration.value);
        return one_to_four(&parts)
            .map(|values| {
                SIDES
                    .iter()
                    .zip(values)
                    .map(|(side, value)| Declaration {
                        property: format!("border-{side}-{}", &declaration.property[7..]),
                        value: value.to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    let prefix = match declaration.property.as_str() {
        "margin" => "margin",
        "padding" => "padding",
        "inset" => "inset",
        "border-radius" => return expand_border_radius(&declaration.value),
        "animation" => {
            return crate::animation::expand_shorthand(&declaration.value).unwrap_or_else(|| {
                log::debug!(
                    "dropping invalid animation shorthand value `{}`",
                    declaration.value
                );
                Vec::new()
            });
        }
        _ => return vec![declaration.clone()],
    };
    let parts = split_components(&declaration.value);
    let Some(sides) = one_to_four(&parts) else {
        log::debug!(
            "dropping invalid {prefix} shorthand value `{}`",
            declaration.value
        );
        return Vec::new();
    };
    log::trace!("expanding {prefix} shorthand `{}`", declaration.value);
    SIDES
        .iter()
        .zip(sides)
        .map(|(side, value)| Declaration {
            // `inset` sets the bare side properties.
            property: if prefix == "inset" {
                (*side).to_owned()
            } else {
                format!("{prefix}-{side}")
            },
            value: value.to_owned(),
        })
        .collect()
}

fn legacy_box_align(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "start" => "flex-start".to_owned(),
        "end" => "flex-end".to_owned(),
        "center" | "baseline" | "stretch" => value.trim().to_ascii_lowercase(),
        _ => value.to_owned(),
    }
}

fn legacy_box_pack(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "start" => "flex-start".to_owned(),
        "end" => "flex-end".to_owned(),
        "center" => "center".to_owned(),
        "justify" => "space-between".to_owned(),
        _ => value.to_owned(),
    }
}

/// Expands the supported color-only portion of `background`. Image, repeat,
/// position, and sizing components are ignored; the shorthand still resets
/// the background color when it contains no supported color.
fn expand_background(value: &str) -> Vec<Declaration> {
    let color = if CSS_WIDE_KEYWORDS
        .iter()
        .any(|keyword| value.trim().eq_ignore_ascii_case(keyword))
    {
        value.trim()
    } else {
        split_components(value)
            .into_iter()
            .find(|part| crate::parse_color(part).is_some())
            .unwrap_or("transparent")
    };
    vec![Declaration {
        property: "background-color".to_owned(),
        value: color.to_owned(),
    }]
}

/// Expands the width, style, and color components of the outline shorthand.
fn expand_outline(value: &str) -> Vec<Declaration> {
    let trimmed = value.trim();
    let mut components = ["medium", "none", "currentcolor"];
    if CSS_WIDE_KEYWORDS
        .iter()
        .any(|keyword| trimmed.eq_ignore_ascii_case(keyword))
    {
        components = [trimmed; 3];
    } else {
        let mut seen = [false; 3];
        for token in split_components(trimmed) {
            let lower = token.to_ascii_lowercase();
            let index = if matches!(
                lower.as_str(),
                "none" | "auto" | "solid" | "dashed" | "dotted" | "double"
            ) {
                1
            } else if matches!(lower.as_str(), "thin" | "medium" | "thick")
                || crate::values::parse_length(&lower).is_some_and(|length| length.value >= 0.0)
            {
                0
            } else if lower == "currentcolor" || crate::parse_color(token).is_some() {
                2
            } else {
                return Vec::new();
            };
            if seen[index] {
                return Vec::new();
            }
            seen[index] = true;
            components[index] = token;
        }
    }
    ["outline-width", "outline-style", "outline-color"]
        .into_iter()
        .zip(components)
        .map(|(property, value)| Declaration {
            property: property.to_owned(),
            value: value.to_owned(),
        })
        .collect()
}

/// The longhands the `font` shorthand sets, in the order it lists them.
const FONT_LONGHANDS: [&str; 7] = [
    "font-style",
    "font-variant",
    "font-weight",
    "font-stretch",
    "font-size",
    "line-height",
    "font-family",
];

/// Expands `font: [style || variant || weight || stretch]? size[/line-height]? family`.
/// Returns `None` for invalid values, including the unsupported system font
/// keywords.
fn expand_font(value: &str) -> Option<Vec<Declaration>> {
    let value = value.trim();
    let longhands = |values: [&str; 7]| {
        FONT_LONGHANDS
            .into_iter()
            .zip(values)
            .map(|(property, value)| Declaration {
                property: property.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    };
    if CSS_WIDE_KEYWORDS
        .iter()
        .any(|keyword| value.eq_ignore_ascii_case(keyword))
    {
        return Some(longhands([value; 7]));
    }
    // Everything from the first quote or comma on belongs to the family
    // list, so only the part before it is split into keyword tokens.
    let prefix_end = value.find(['"', '\'', ',']).unwrap_or(value.len());
    let mut values = ["normal"; 7];
    let mut seen = [false; 4];
    let mut rest = value;
    loop {
        let trimmed = rest.trim_start();
        let token_end = trimmed
            .find(|c: char| c.is_whitespace())
            .unwrap_or(trimmed.len());
        let offset = value.len() - trimmed.len();
        if token_end == 0 || offset >= prefix_end {
            return None;
        }
        let token = &trimmed[..token_end];
        let lower = token.to_ascii_lowercase();
        let slot = match lower.as_str() {
            "normal" => {
                rest = &trimmed[token_end..];
                continue;
            }
            "italic" | "oblique" => Some(0),
            "small-caps" => Some(1),
            "bold" | "bolder" | "lighter" => Some(2),
            "ultra-condensed" | "extra-condensed" | "condensed" | "semi-condensed"
            | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded" => Some(3),
            _ if crate::fonts::parse_font_weight(token).is_some()
                && token.parse::<f32>().is_ok() =>
            {
                Some(2)
            }
            _ => None,
        };
        match slot {
            Some(slot) if !seen[slot] => {
                seen[slot] = true;
                values[slot] = token;
                rest = &trimmed[token_end..];
            }
            Some(_) => return None,
            None => {
                rest = trimmed;
                break;
            }
        }
    }
    // The size, optionally followed by `/ line-height`, then the family list.
    let size_end = rest
        .find(|c: char| c.is_whitespace() || c == '/')
        .unwrap_or(rest.len());
    let size = &rest[..size_end];
    // The size is validated here; the cascade computes it.
    crate::values::compute_font_size(size, 16.0, 16.0)?;
    values[4] = size;
    let mut after = rest[size_end..].trim_start();
    if let Some(line_height) = after.strip_prefix('/') {
        let line_height = line_height.trim_start();
        let end = line_height
            .find(|c: char| c.is_whitespace())
            .unwrap_or(line_height.len());
        if end == 0 {
            return None;
        }
        values[5] = &line_height[..end];
        after = line_height[end..].trim_start();
    }
    if after.is_empty() {
        return None;
    }
    values[6] = after;
    log::trace!("expanding font shorthand `{value}`");
    Some(longhands(values))
}

/// Expands `overflow: <x> [<y>]` into `overflow-x` and `overflow-y`; one
/// value sets both.
fn expand_overflow(value: &str) -> Vec<Declaration> {
    let parts = split_components(value);
    let (x, y) = match parts.as_slice() {
        [both] => (*both, *both),
        [x, y] => (*x, *y),
        _ => {
            log::debug!("dropping invalid overflow shorthand value `{value}`");
            return Vec::new();
        }
    };
    vec![
        Declaration {
            property: "overflow-x".to_owned(),
            value: x.to_owned(),
        },
        Declaration {
            property: "overflow-y".to_owned(),
            value: y.to_owned(),
        },
    ]
}

/// Expands the numeric and keyword forms of the `flex` shorthand.
fn expand_flex(value: &str) -> Vec<Declaration> {
    if CSS_WIDE_KEYWORDS
        .iter()
        .any(|keyword| value.trim().eq_ignore_ascii_case(keyword))
    {
        return ["flex-grow", "flex-shrink", "flex-basis"]
            .into_iter()
            .map(|property| Declaration {
                property: property.to_owned(),
                value: value.trim().to_owned(),
            })
            .collect();
    }
    let parts = split_components(value);
    let values = match parts.as_slice() {
        ["none"] => ["0", "0", "auto"],
        ["auto"] => ["1", "1", "auto"],
        [grow] if grow.parse::<f32>().is_ok() => [*grow, "1", "0%"],
        [grow, shrink] if grow.parse::<f32>().is_ok() && shrink.parse::<f32>().is_ok() => {
            [*grow, *shrink, "0%"]
        }
        [grow, shrink, basis] if grow.parse::<f32>().is_ok() && shrink.parse::<f32>().is_ok() => {
            [*grow, *shrink, *basis]
        }
        _ => return Vec::new(),
    };
    ["flex-grow", "flex-shrink", "flex-basis"]
        .into_iter()
        .zip(values)
        .map(|(property, value)| Declaration {
            property: property.to_owned(),
            value: value.to_owned(),
        })
        .collect()
}

fn expand_border(declaration: &Declaration) -> Vec<Declaration> {
    let value = declaration.value.trim();
    let mut values = ["medium", "none", "currentcolor"];
    if CSS_WIDE_KEYWORDS.contains(&value) {
        values = [value; 3];
    } else {
        let mut seen = [false; 3];
        for token in split_components(value) {
            let lower = token.to_ascii_lowercase();
            let index = if matches!(
                lower.as_str(),
                "none"
                    | "hidden"
                    | "solid"
                    | "dashed"
                    | "dotted"
                    | "double"
                    | "groove"
                    | "ridge"
                    | "inset"
                    | "outset"
            ) {
                1
            } else if matches!(lower.as_str(), "thin" | "medium" | "thick")
                || crate::values::parse_length(&lower).is_some_and(|v| v.value >= 0.0)
            {
                0
            } else if lower == "currentcolor" || crate::parse_color(token).is_some() {
                2
            } else {
                return Vec::new();
            };
            if seen[index] {
                return Vec::new();
            }
            seen[index] = true;
            values[index] = token;
        }
        if !seen.iter().any(|v| *v) {
            return Vec::new();
        }
    }
    let side = &declaration.property[declaration.property.len().min(7)..];
    let sides: &[&str] = if declaration.property == "border" {
        &SIDES
    } else {
        std::slice::from_ref(&side)
    };
    sides
        .iter()
        .flat_map(|side| {
            ["width", "style", "color"]
                .iter()
                .zip(values)
                .map(move |(part, value)| Declaration {
                    property: format!("border-{side}-{part}"),
                    value: value.to_owned(),
                })
        })
        .collect()
}

/// Applies the one-to-four value rule, returning values for top, right,
/// bottom, and left (or top-left, top-right, bottom-right, and bottom-left).
fn one_to_four<'a>(parts: &[&'a str]) -> Option<[&'a str; 4]> {
    match *parts {
        [all] => Some([all, all, all, all]),
        [vertical, horizontal] => Some([vertical, horizontal, vertical, horizontal]),
        [top, horizontal, bottom] => Some([top, horizontal, bottom, horizontal]),
        [top, right, bottom, left] => Some([top, right, bottom, left]),
        _ => None,
    }
}

/// Expands `border-radius` to its four corner longhands.
fn expand_border_radius(value: &str) -> Vec<Declaration> {
    let longhands = |values: [String; 4]| {
        CORNERS
            .iter()
            .zip(values)
            .map(|(corner, value)| Declaration {
                property: format!("border-{corner}-radius"),
                value,
            })
            .collect()
    };
    let trimmed = value.trim();
    if CSS_WIDE_KEYWORDS
        .iter()
        .any(|keyword| trimmed.eq_ignore_ascii_case(keyword))
    {
        return longhands([0; 4].map(|_| trimmed.to_owned()));
    }
    let (horizontal, vertical) = match trimmed.split_once('/') {
        Some((horizontal, vertical)) => (horizontal, Some(vertical)),
        None => (trimmed, None),
    };
    let horizontal = one_to_four(&split_components(horizontal));
    let vertical = vertical.map(|vertical| one_to_four(&split_components(vertical)));
    let values = match (horizontal, vertical) {
        (Some(horizontal), None) => horizontal.map(str::to_owned),
        (Some(horizontal), Some(Some(vertical))) => {
            let mut values = [0; 4].map(|_| String::new());
            for (index, value) in values.iter_mut().enumerate() {
                *value = format!("{} {}", horizontal[index], vertical[index]);
            }
            values
        }
        _ => {
            log::debug!("dropping invalid border-radius shorthand value `{value}`");
            return Vec::new();
        }
    };
    log::trace!("expanding border-radius shorthand `{value}`");
    longhands(values)
}

/// Splits a value into whitespace-separated components, keeping parenthesized
/// groups such as `calc(1px + 2px)` intact.
pub(crate) fn split_components(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0_usize;
    let mut start = None;
    for (index, character) in value.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if character.is_whitespace() && depth == 0 {
            if let Some(begin) = start.take() {
                parts.push(&value[begin..index]);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(begin) = start {
        parts.push(&value[begin..]);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expanded(property: &str, value: &str) -> Vec<(String, String)> {
        expand(&Declaration {
            property: property.into(),
            value: value.into(),
        })
        .into_iter()
        .map(|declaration| (declaration.property, declaration.value))
        .collect()
    }

    #[test]
    fn box_shorthands_follow_the_one_to_four_value_rule() {
        assert_eq!(
            expanded("margin", "1px 2px 3px"),
            [
                ("margin-top".into(), "1px".into()),
                ("margin-right".into(), "2px".into()),
                ("margin-bottom".into(), "3px".into()),
                ("margin-left".into(), "2px".into()),
            ]
        );
        assert_eq!(expanded("padding", "1px 2px 3px 4px 5px"), []);
        assert_eq!(
            expanded("inset", "0 auto"),
            [
                ("top".into(), "0".into()),
                ("right".into(), "auto".into()),
                ("bottom".into(), "0".into()),
                ("left".into(), "auto".into()),
            ]
        );
        assert_eq!(expanded("color", "red"), [("color".into(), "red".into())]);
    }

    #[test]
    fn css_wide_keywords_apply_to_every_longhand() {
        assert!(
            expanded("margin", "inherit")
                .iter()
                .all(|(_, value)| value == "inherit")
        );
        let radius = expanded("border-radius", "initial");
        assert_eq!(radius.len(), 4);
        assert!(radius.iter().all(|(_, value)| value == "initial"));
    }

    #[test]
    fn border_radius_expands_to_corner_longhands() {
        assert_eq!(
            expanded("border-radius", "3px"),
            [
                ("border-top-left-radius".into(), "3px".into()),
                ("border-top-right-radius".into(), "3px".into()),
                ("border-bottom-right-radius".into(), "3px".into()),
                ("border-bottom-left-radius".into(), "3px".into()),
            ]
        );
        assert_eq!(
            expanded("border-radius", "1px 2px 3px / 4px 5px"),
            [
                ("border-top-left-radius".into(), "1px 4px".into()),
                ("border-top-right-radius".into(), "2px 5px".into()),
                ("border-bottom-right-radius".into(), "3px 4px".into()),
                ("border-bottom-left-radius".into(), "2px 5px".into()),
            ]
        );
        assert_eq!(expanded("border-radius", "1px / "), []);
        assert_eq!(expanded("border-radius", "1px 2px 3px 4px 5px"), []);
    }

    #[test]
    fn webkit_flexbox_and_animation_aliases_normalize_to_standard_properties() {
        assert_eq!(
            expanded("-webkit-align-items", "center"),
            [("align-items".into(), "center".into())]
        );
        assert_eq!(
            expanded("-webkit-box-align", "start"),
            [("align-items".into(), "flex-start".into())]
        );
        assert_eq!(
            expanded("-webkit-box-pack", "justify"),
            [("justify-content".into(), "space-between".into())]
        );
        assert_eq!(
            expanded("-webkit-justify-content", "center"),
            [("justify-content".into(), "center".into())]
        );
        let animation = expanded("-webkit-animation", "fade 1s linear");
        assert_eq!(animation.len(), 8);
        assert_eq!(animation[0], ("animation-name".into(), "fade".into()));
    }

    #[test]
    fn supported_css_property_shorthands_expand_to_longhands() {
        assert_eq!(
            expanded("background", "url(image.png) center red"),
            [("background-color".into(), "red".into())]
        );
        assert_eq!(
            expanded("outline", "2px dashed blue"),
            [
                ("outline-width".into(), "2px".into()),
                ("outline-style".into(), "dashed".into()),
                ("outline-color".into(), "blue".into()),
            ]
        );
        assert_eq!(
            expanded("flex", "2"),
            [
                ("flex-grow".into(), "2".into()),
                ("flex-shrink".into(), "1".into()),
                ("flex-basis".into(), "0%".into()),
            ]
        );
        assert_eq!(
            expanded("text-decoration", "underline"),
            [("text-decoration-line".into(), "underline".into())]
        );
        assert_eq!(
            expanded("-webkit-box-flex", "2"),
            [("flex-grow".into(), "2".into())]
        );
        assert_eq!(
            expanded("-ms-transform", "rotate(45deg)"),
            [("transform".into(), "rotate(45deg)".into())]
        );
        assert_eq!(
            expanded("-webkit-transform", "scale(2)"),
            [("transform".into(), "scale(2)".into())]
        );
        assert_eq!(
            expanded("-webkit-transform-origin", "0 0"),
            [("transform-origin".into(), "0 0".into())]
        );
        assert_eq!(
            expanded("-ms-filter", "x"),
            [("-ms-filter".into(), "x".into())]
        );
        assert_eq!(
            expanded("overflow", "hidden auto"),
            [
                ("overflow-x".into(), "hidden".into()),
                ("overflow-y".into(), "auto".into()),
            ]
        );
        assert_eq!(
            expanded("-webkit-flex", "2"),
            [
                ("flex-grow".into(), "2".into()),
                ("flex-shrink".into(), "1".into()),
                ("flex-basis".into(), "0%".into()),
            ]
        );
    }
}

#[cfg(test)]
mod font_tests {
    use super::*;

    fn font(value: &str) -> Vec<(String, String)> {
        expand(&Declaration {
            property: "font".into(),
            value: value.into(),
        })
        .into_iter()
        .map(|declaration| (declaration.property, declaration.value))
        .collect()
    }

    fn get<'a>(expanded: &'a [(String, String)], property: &str) -> &'a str {
        &expanded
            .iter()
            .find(|(name, _)| name == property)
            .unwrap_or_else(|| panic!("{property} missing from {expanded:?}"))
            .1
    }

    #[test]
    fn font_shorthand_sets_every_component_and_resets_the_rest() {
        let full = font("italic small-caps bold 12px/1.5 \"Helvetica Neue\", Arial, sans-serif");
        assert_eq!(get(&full, "font-style"), "italic");
        assert_eq!(get(&full, "font-variant"), "small-caps");
        assert_eq!(get(&full, "font-weight"), "bold");
        assert_eq!(get(&full, "font-stretch"), "normal");
        assert_eq!(get(&full, "font-size"), "12px");
        assert_eq!(get(&full, "line-height"), "1.5");
        assert_eq!(
            get(&full, "font-family"),
            "\"Helvetica Neue\", Arial, sans-serif"
        );

        let minimal = font("2em serif");
        assert_eq!(minimal.len(), 7);
        assert_eq!(get(&minimal, "font-weight"), "normal");
        assert_eq!(get(&minimal, "line-height"), "normal");
        assert_eq!(get(&minimal, "font-size"), "2em");

        let spaced = font("600 large / 20px monospace");
        assert_eq!(get(&spaced, "font-weight"), "600");
        assert_eq!(get(&spaced, "font-size"), "large");
        assert_eq!(get(&spaced, "line-height"), "20px");

        assert_eq!(get(&font("inherit"), "font-family"), "inherit");
    }

    #[test]
    fn invalid_font_shorthands_are_dropped() {
        assert!(font("bold").is_empty());
        assert!(font("12px").is_empty());
        assert!(font("bold bold 12px serif").is_empty());
        assert!(font("caption").is_empty());
        assert!(font("\"Arial\" 12px").is_empty());
    }
}

#[cfg(test)]
mod border_tests {
    use super::*;

    fn expand_value(property: &str, value: &str) -> Vec<Declaration> {
        expand(&Declaration {
            property: property.into(),
            value: value.into(),
        })
    }

    #[test]
    fn borders_expand_before_cascade_and_reset_omitted_components() {
        let result = expand_value("border-left", "rgb(10, 20, 30) solid 2px");
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].property, "border-left-width");
        assert_eq!(result[0].value, "2px");
        assert_eq!(result[2].value, "rgb(10, 20, 30)");
        let reset = expand_value("border", "solid");
        assert_eq!(reset.len(), 12);
        assert_eq!(reset[0].value, "medium");
        assert_eq!(reset[2].value, "currentcolor");
        assert!(expand_value("border", "solid dashed").is_empty());
        assert!(expand_value("border", "-2px solid").is_empty());
        assert!(expand_value("border", "2% solid").is_empty());
        assert_eq!(expand_value("border", "inherit")[11].value, "inherit");
        assert_eq!(expand_value("border-color", "red blue")[3].value, "blue");
    }
}
