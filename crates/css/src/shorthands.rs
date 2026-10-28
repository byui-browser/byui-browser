//! Shorthand property expansion (CSS Cascading and Inheritance 4 §1.2).
//!
//! Shorthands are expanded to their longhands before the cascade so that, for
//! example, an author `margin: 0` competes with a user-agent `margin-top`.

use crate::Declaration;

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// Expands one declaration into the longhand declarations it sets.
///
/// Only the box shorthands `margin` and `padding` are expanded today, using
/// the one-to-four value rule. Other declarations are returned unchanged. An
/// invalid `margin` or `padding` value expands to nothing, matching CSS's rule
/// that invalid declarations are dropped. CSS-wide keywords such as `inherit`
/// are copied to every longhand.
pub fn expand(declaration: &Declaration) -> Vec<Declaration> {
    let prefix = match declaration.property.as_str() {
        "margin" => "margin",
        "padding" => "padding",
        _ => return vec![declaration.clone()],
    };
    let parts = split_components(&declaration.value);
    let sides = match parts.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return Vec::new(),
    };
    SIDES
        .iter()
        .zip(sides)
        .map(|(side, value)| Declaration {
            property: format!("{prefix}-{side}"),
            value: value.to_owned(),
        })
        .collect()
}

/// Splits a value into whitespace-separated components, keeping parenthesized
/// groups such as `calc(1px + 2px)` intact.
fn split_components(value: &str) -> Vec<&str> {
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
    }
}
