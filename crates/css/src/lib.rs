//! CSS parsing, cascade, inheritance, and computed styles.
//!
//! The parser is intentionally forgiving, like a browser parser: it keeps
//! valid rules when a neighboring rule is malformed and reports malformed
//! pieces as [`ParseError`] values.

#![forbid(unsafe_code)]

mod cascade;
mod model;
mod parser;
mod specificity;

/// Parsing of CSS selector syntax into a structured abstract syntax tree.
pub mod selector;

pub use cascade::style_document;
pub use model::{
    AtRule, ComputedStyles, Declaration, Dom, ParseError, Rule, Selector, Specificity, Stylesheet,
};
pub use parser::{parse_stylesheet, parse_stylesheet_with_errors};
pub use selector::{
    AttributeOperator, AttributeSelector, Combinator, ComplexSelector, CompoundSelector,
    PseudoSelector, SelectorList, SelectorParseError, SimpleSelector, TypeSelector,
    parse_selector_list,
};
pub use specificity::specificity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comments_strings_functions_and_important_values() {
        let sheet = parse_stylesheet(
            r#"/* comment */ body, .hero { background: url("a;b.png"); content: "}"; color: rgb(1, 2, 3) !important; }"#,
        );
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].declarations.len(), 3);
        assert!(sheet.errors.is_empty());
    }

    #[test]
    fn parses_at_rules_and_nested_rules() {
        let sheet = parse_stylesheet(
            "@media screen and (min-width: 10px) { body { color: red; } } @import url('x.css');",
        );
        assert_eq!(sheet.at_rules.len(), 2);
        assert_eq!(sheet.at_rules[0].name, "media");
        assert_eq!(sheet.at_rules[0].rules.len(), 1);
        assert_eq!(sheet.at_rules[1].prelude, "url('x.css')");
    }

    #[test]
    fn recovers_from_bad_declaration() {
        let (sheet, errors) =
            parse_stylesheet_with_errors("p { color: red; broken; background: blue; }");
        assert_eq!(sheet.rules[0].declarations.len(), 2);
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn specificity_orders_ids_classes_and_elements() {
        assert!(specificity(&Selector("#main".into())) > specificity(&Selector(".card".into())));
        assert!(specificity(&Selector(".card".into())) > specificity(&Selector("div".into())));
        assert_eq!(
            specificity(&Selector("div.card#main".into())),
            Specificity(1, 1, 1)
        );
    }

    #[test]
    fn cascades_matches_and_inlines() {
        let document = html::parse_raw_html(
            "<div><p id='target' class='intro' style='color: black'>Hello</p></div>".into(),
        );
        let stylesheet =
            parse_stylesheet("p { color: green; } .intro { color: red; font-size: 20px; }");
        let styles = style_document(&document, &[stylesheet]);
        let paragraph = document
            .get_element_by_id("target")
            .expect("paragraph node id");
        let declarations = styles
            .styles
            .iter()
            .find(|(id, _)| id.index() as usize == paragraph.index())
            .expect("computed paragraph");
        assert!(declarations.1.contains(&Declaration {
            property: "color".into(),
            value: "black".into()
        }));
        assert!(declarations.1.contains(&Declaration {
            property: "font-size".into(),
            value: "20px".into()
        }));
    }
}
