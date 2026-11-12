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
/// one-to-four value rule. `border-radius` is expanded to the four
/// `border-*-radius` longhands (CSS Backgrounds 3 §5.1): the values before an
/// optional `/` are the horizontal radii and the values after it the vertical
/// radii, each following the one-to-four value rule starting at the top-left
/// corner. A longhand gets `"<horizontal> <vertical>"`, or just the
/// horizontal radius when there is no `/`. Values are not validated here.
///
/// `animation` is expanded to the eight `animation-*` longhands, each a
/// comma-separated list with one entry per animation (CSS Animations 1
/// §3.10).
///
/// Other declarations are returned unchanged. An invalid shorthand value
/// expands to nothing, matching CSS's rule that invalid declarations are
/// dropped. CSS-wide keywords such as `inherit` are copied to every longhand.
pub fn expand(declaration: &Declaration) -> Vec<Declaration> {
    let prefix = match declaration.property.as_str() {
        "margin" => "margin",
        "padding" => "padding",
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
            property: format!("{prefix}-{side}"),
            value: value.to_owned(),
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
}
