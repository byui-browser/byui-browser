//! Shorthand property expansion (CSS Cascading and Inheritance 4 §1.2).
//!
//! Shorthands are expanded to their longhands before the cascade so that,
//! for example, an author `margin: 0` competes with a user-agent
//! `margin-top`.

use crate::Declaration;

const SIDES: [&str; 4] = ["top", "right", "bottom", "left"];

/// Expands one declaration into its longhand declarations.
///
/// The supported box shorthands are `margin` and `padding`, using the
/// one-to-four value rule. Other declarations are returned unchanged.
///
/// Invalid `margin` or `padding` values are dropped. CSS-wide keywords such
/// as `inherit` are copied to every longhand.
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
        _ => {
            log::debug!(
                "dropping invalid {prefix} shorthand value `{}`",
                declaration.value
            );
            return Vec::new();
        }
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

/// Splits a value into whitespace-separated components, preserving
/// parenthesized groups such as `calc(1px + 2px)` as one component.
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