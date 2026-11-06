//! Selector specificity (Selectors Level 4 §16).

use crate::selector::{
    ComplexSelector, CompoundSelector, SelectorList, SimpleSelector, parse_selector_list,
};
use crate::stylesheet::{Selector, Specificity};

/// Returns the specificity of a selector, written as `(ids, classes, types)`.
///
/// The text is parsed as a selector list; for a list the highest specificity
/// of its entries is returned. Invalid selectors have specificity `(0, 0, 0)`.
/// `:is()`, `:not()`, `:has()`, and `:nth-child(… of S)` take the specificity
/// of their most specific argument, and `:where()` contributes nothing.
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

/// The specificity of one complex selector: the sum over its compounds.
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

/// Pseudo-elements that may be written with a single colon (CSS 2).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Selector, Specificity};

    fn of(selector: &str) -> Specificity {
        specificity(&Selector(selector.into()))
    }

    #[test]
    fn specificity_orders_ids_classes_and_elements() {
        assert!(of("#main") > of(".card"));
        assert!(of(".card") > of("div"));
        assert_eq!(of("div.card#main"), Specificity(1, 1, 1));
        assert_eq!(of("ul li > a[href]:hover::before"), Specificity(0, 2, 4));
        assert_eq!(of("*"), Specificity(0, 0, 0));
    }

    #[test]
    fn logical_pseudo_classes_use_their_arguments() {
        assert_eq!(of(":is(#a, .b) p"), Specificity(1, 0, 1));
        assert_eq!(of(":where(#a, .b) p"), Specificity(0, 0, 1));
        assert_eq!(of("a:not(.x, #y)"), Specificity(1, 0, 1));
        assert_eq!(of("li:nth-child(2n of .item)"), Specificity(0, 2, 1));
        assert_eq!(of("p:before"), Specificity(0, 0, 2));
    }

    #[test]
    fn invalid_selectors_have_zero_specificity() {
        assert_eq!(of("div >"), Specificity(0, 0, 0));
    }
}
