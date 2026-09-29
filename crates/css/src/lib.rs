//! CSS parser, stylesheet management, cascade, and computed values.
//!
//! **Owning team**: CSS Engine Team
//!
//! Planned interface (architecture §3.2):
//! `style_document(dom, stylesheets) -> ComputedStyles`

#![forbid(unsafe_code)]

use common::ids::NodeId;

/// A selector as written in the stylesheet, e.g. `div.card#main`.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector(pub String);

/// Cascade specificity as `(ids, classes, tags)`. Derives `Ord` so that
/// higher specificity compares greater.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity(pub u32, pub u32, pub u32);

/// A `property: value` pair.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub property: String,
    pub value: String,
}

/// One rule: a selector and its declarations.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
}

/// A parsed stylesheet.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

/// Placeholder for the DOM handed over by the HTML team.
///
/// This will move to `common` once the HTML team fixes the real DOM shape.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Dom {
    pub nodes: Vec<NodeId>,
}

/// Computed style for every node, keyed by node.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComputedStyles {
    pub styles: Vec<(NodeId, Vec<Declaration>)>,
}

/// Parses stylesheet text into rules.
///
/// Currently only handles the empty stylesheet.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let input = remove_comments(input);
    let mut rules = Vec::new();

    for rule_text in input.split('}') {
        let Some((selector, declarations)) = rule_text.split_once('{') else {
            continue;
        };

        let selector = selector.trim();
        if selector.is_empty() {
            continue;
        }

        let declarations = declarations
            .split(';')
            .filter_map(|declaration| {
                let (property, value) = declaration.split_once(':')?;
                let property = property.trim();
                let value = value.trim();
                if property.is_empty() || value.is_empty() {
                    return None;
                }
                Some(Declaration {
                    property: property.to_owned(),
                    value: value.to_owned(),
                })
            })
            .collect();

        rules.push(Rule {
            selector: Selector(selector.to_owned()),
            declarations,
        });
    }

    Stylesheet { rules }
}

/// Computes the specificity of a selector.
///
/// Currently only handles the empty selector.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn specificity(selector: &Selector) -> Specificity {
    let ids = selector.0.matches('#').count() as u32;
    let classes = selector.0.matches('.').count() as u32;
    let tags = selector
        .0
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter(|part| !part.is_empty())
        .filter(|part| !selector.0.contains(&format!("#{part}")))
        .filter(|part| !selector.0.contains(&format!(".{part}")))
        .count() as u32;

    Specificity(ids, classes, tags)
}

fn remove_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = remaining.find("/*") {
        result.push_str(&remaining[..start]);
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("*/") else {
            return result;
        };
        remaining = &after_start[end + 2..];
    }

    result.push_str(remaining);
    result
}

/// Resolves the cascade for every node in `dom` (architecture §3.2).
///
/// Currently only handles an empty document with no stylesheets.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// signature however your crate's public API needs.
pub fn style_document(dom: &Dom, stylesheets: &[Stylesheet]) -> ComputedStyles {
    if dom.nodes.is_empty() && stylesheets.is_empty() {
        return ComputedStyles::default();
    }
    todo!(
        "TODO(css): cascade {} sheets over {} nodes",
        stylesheets.len(),
        dom.nodes.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stylesheet_has_no_rules() {
        assert!(parse_stylesheet("").rules.is_empty());
        assert!(parse_stylesheet("   \n").rules.is_empty());
    }

    #[test]
    fn empty_selector_has_zero_specificity() {
        assert_eq!(specificity(&Selector(String::new())), Specificity(0, 0, 0));
    }

    #[test]
    fn specificity_orders_ids_over_classes_over_tags() {
        // Pure type behavior, no parsing involved.
        assert!(Specificity(1, 0, 0) > Specificity(0, 9, 9));
        assert!(Specificity(0, 1, 0) > Specificity(0, 0, 9));
    }

    #[test]
    fn parses_body_background_as_blue() {
        let sheet = parse_stylesheet("body { background-color: blue; }");

        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector, Selector("body".into()));
        assert_eq!(
            sheet.rules[0].declarations,
            vec![Declaration {
                property: "background-color".into(),
                value: "blue".into(),
            }]
        );
    }

    #[test]
    fn empty_document_has_no_styles() {
        assert_eq!(
            style_document(&Dom::default(), &[]),
            ComputedStyles::default()
        );
    }
}
