//! Integration tests for CSS selector text in stylesheet parsing.
//!
//! These test the four selector examples from class. The current parser
//! preserves selectors as strings rather than building a structured selector tree.

use css::{parse_stylesheet, Declaration, Selector};

/// Check that a rule's selector and declaration are parsed as expected.
fn assert_parses_selector(selector: &str) {
    let input = format!("{selector} {{ color: red; }}");
    let stylesheet = parse_stylesheet(&input);

    assert_eq!(stylesheet.rules.len(), 1, "expected exactly one CSS rule");
    assert_eq!(
        stylesheet.rules[0].selector,
        Selector(selector.to_string()),
        "selector text was not preserved"
    );
    assert_eq!(
        stylesheet.rules[0].declarations,
        vec![Declaration {
            property: "color".to_string(),
            value: "red".to_string(),
        }]
    );
}

#[test]
fn parses_div_type_selector() {
    assert_parses_selector("div");
}

#[test]
fn parses_div_and_footer_selector_list() {
    assert_parses_selector("div, footer");
}

#[test]
fn parses_direct_child_selector() {
    assert_parses_selector("div > footer");
}

#[test]
fn parses_nth_child_pseudo_class() {
    assert_parses_selector("div:nth-child(2)");
}
