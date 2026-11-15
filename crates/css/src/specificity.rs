//! Selector specificity (Selectors Level 4 §16).

use crate::selector::{
    ComplexSelector, CompoundSelector, SelectorList, SimpleSelector, parse_selector_list,
};
use crate::stylesheet::{Selector, Specificity};

/// Returns the specificity of a selector as `(ids, classes, types)`.
///
/// For a selector list, the highest specificity of its entries is returned.
/// `:is()`, `:not()`, `:has()`, and `:nth-child(... of S)` use the specificity
/// of their most specific argument. `:where()` contributes zero.
pub fn specificity(selector: &Selector) -> Specificity {
    match parse_selector_list(&selector.0) {
        Ok(list) => list_specificity(&list),
        Err(error) => {
            log::debug!("specificity of invalid selector `{}`: {error}", selector.0);
            Specificity(0, 0, 0)
        }
    }
}

/// The specificity of the most specific selector in `list`.
pub(crate) fn list_specificity(list: &SelectorList) -> Specificity {
    list.selectors
        .iter()
        .map(complex_specificity)
        .max()
        .unwrap_or(Specificity(0, 0, 0))
}

/// The specificity of one complex selector.
pub(crate) fn complex_specificity(selector: &ComplexSelector) -> Specificity {
    selector
        .compounds
        .iter()
        .map(compound_specificity)
        .fold(Specificity(0, 0, 0), add)
}

fn add(left: Specificity, right: Specificity) -> Specificity {
    Specificity(
        left.0.saturating_add(right.0),
        left.1.saturating_add(right.1),
        left.2.saturating_add(right.2),
    )
}

/// Pseudo-elements that may be written with a single colon.
pub(crate) fn is_legacy_pseudo_element(name: &str) -> bool {
    ["before", "after", "first-line", "first-letter"]
        .iter()
        .any(|legacy| name.eq_ignore_ascii_case(legacy))
}

fn compound_specificity(compound: &CompoundSelector) -> Specificity {
    let mut result = Specificity(0, 0, 0);

    if compound
        .type_selector
        .as_ref()
        .is_some_and(|type_selector| type_selector.name != "*")
    {
        result.2 += 1;
    }

    for simple in &compound.simples {
        let part = match simple {
            SimpleSelector::Id(_) => Specificity(1, 0, 0),

            SimpleSelector::Class(_) | SimpleSelector::Attribute(_) => Specificity(0, 1, 0),

            SimpleSelector::Nesting => Specificity(0, 0, 0),

            SimpleSelector::Pseudo(pseudo) => {
                let nested = pseudo
                    .nested_selectors
                    .as_deref()
                    .map_or(Specificity(0, 0, 0), list_specificity);

                let name = pseudo.name.to_ascii_lowercase();

                if pseudo.element || is_legacy_pseudo_element(&name) {
                    add(Specificity(0, 0, 1), nested)
                } else {
                    match name.as_str() {
                        "where" => Specificity(0, 0, 0),

                        "is" | "matches" | "-webkit-any" | "-moz-any" | "not" | "has" => nested,

                        "nth-child" | "nth-last-child" => add(Specificity(0, 1, 0), nested),

                        _ => Specificity(0, 1, 0),
                    }
                }
            }
        };

        result = add(result, part);
    }

    result
}