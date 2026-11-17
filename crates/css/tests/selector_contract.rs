//! Public integration tests for CSS selector parsing.

use css::parse_selector_list;

#[test]
fn parses_type_class_and_id_selectors() {
    let parsed = parse_selector_list("article, .card, #main").expect("valid basic selectors");

    assert_eq!(parsed.selectors.len(), 3);
}

#[test]
fn parses_standard_and_custom_element_selectors() {
    let parsed = parse_selector_list("article, my-widget, *").expect("valid element selectors");

    assert_eq!(parsed.selectors.len(), 3);
}

#[test]
fn parses_attribute_selectors() {
    let parsed =
        parse_selector_list("[hidden], [type=\"text\"], [data-state~=\"active\"], [lang|=\"en\"]")
            .expect("valid attribute selectors");

    assert_eq!(parsed.selectors.len(), 4);
}

#[test]
fn parses_all_selector_combinators() {
    let parsed = parse_selector_list("main article, div > p, h2 + p, h2 ~ p, .card || .column")
        .expect("valid selector combinators");

    assert_eq!(parsed.selectors.len(), 5);
}

#[test]
fn parses_pseudo_classes_and_pseudo_elements() {
    let parsed =
        parse_selector_list(":hover, :first-child, :nth-child(2n+1), ::before, ::placeholder")
            .expect("valid pseudo selectors");

    assert_eq!(parsed.selectors.len(), 5);
}

#[test]
fn parses_nested_logical_selector_functions() {
    let parsed =
        parse_selector_list(":is(article, section), :not(.hidden), :where(#main), :has(> img)")
            .expect("valid logical selector functions");

    assert_eq!(parsed.selectors.len(), 4);
}

#[test]
fn parses_complex_selectors_with_custom_elements() {
    let parsed = parse_selector_list(
        "#app my-widget.active[data-state=\"ready\"] > custom-panel::part(content)",
    )
    .expect("valid complex custom-element selector");

    assert_eq!(parsed.selectors.len(), 1);
}

#[test]
fn rejects_invalid_selector_syntax() {
    assert!(parse_selector_list("div >").is_err());
    assert!(parse_selector_list("[class=").is_err());
}
