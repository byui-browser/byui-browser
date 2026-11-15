//! Public integration tests for CSS cascade and computed styles.

use css::{
    ComputedStyles, Declaration, StyleContext, parse_stylesheet, style_document,
    style_document_with_context,
};
use html::parse_raw_html;

fn value(
    styles: &ComputedStyles,
    document: &html::HTMLDocument,
    id: &str,
    property: &str,
) -> Option<String> {
    let node = document.get_element_by_id(id)?;
    styles.value_for(node, property).map(str::to_owned)
}

#[test]
fn applies_matching_stylesheet_rules() {
    let document = parse_raw_html("<p id='target'>Hello</p>".into());
    let sheet = parse_stylesheet("p { color: red; margin: 5px; }");
    let styles = style_document(&document, &[sheet]);

    assert_eq!(
        value(&styles, &document, "target", "color").as_deref(),
        Some("red")
    );
    assert_eq!(
        value(&styles, &document, "target", "margin-top").as_deref(),
        Some("5px")
    );
}

#[test]
fn inline_styles_override_normal_author_styles() {
    let document = parse_raw_html("<p id='target' style='color: blue'>Hello</p>".into());
    let sheet = parse_stylesheet("p { color: red; }");
    let styles = style_document(&document, &[sheet]);

    assert_eq!(
        value(&styles, &document, "target", "color").as_deref(),
        Some("blue")
    );
}

#[test]
fn important_declarations_override_normal_declarations() {
    let document = parse_raw_html("<p id='target' class='item'>Hello</p>".into());
    let sheet = parse_stylesheet(
        "p { color: green !important; } .item { color: red; } #target { color: blue; }",
    );
    let styles = style_document(&document, &[sheet]);

    assert_eq!(
        value(&styles, &document, "target", "color").as_deref(),
        Some("green")
    );
}

#[test]
fn specificity_and_source_order_determine_the_winner() {
    let document = parse_raw_html("<p id='target' class='item'>Hello</p>".into());
    let sheet =
        parse_stylesheet("p { color: green; } .item { color: red; } #target { color: blue; }");
    let styles = style_document(&document, &[sheet]);

    assert_eq!(
        value(&styles, &document, "target", "color").as_deref(),
        Some("blue")
    );

    let later = parse_stylesheet("p { color: red; color: blue; }");
    let styles = style_document(&document, &[later]);
    assert_eq!(
        value(&styles, &document, "target", "color").as_deref(),
        Some("blue")
    );
}

#[test]
fn later_stylesheets_override_earlier_stylesheets() {
    let document = parse_raw_html("<p id='target'>Hello</p>".into());
    let first = parse_stylesheet("p { color: red; }");
    let second = parse_stylesheet("p { color: blue; }");
    let styles = style_document(&document, &[first, second]);

    assert_eq!(
        value(&styles, &document, "target", "color").as_deref(),
        Some("blue")
    );
}

#[test]
fn inherited_properties_reach_descendants() {
    let document = parse_raw_html(
        "<div id='parent' style='color: red; font-size: 20px'><p id='child'>Hello</p></div>".into(),
    );
    let styles = style_document(&document, &[]);

    assert_eq!(
        value(&styles, &document, "child", "color").as_deref(),
        Some("red")
    );
    assert_eq!(
        value(&styles, &document, "child", "font-size").as_deref(),
        Some("20px")
    );
}

#[test]
fn resolves_css_wide_keywords() {
    let document = parse_raw_html(
        "<div id='parent' style='color: red; margin-left: 5px'><p id='child' style='color: initial; margin-left: inherit'>Hello</p></div>".into()
    );
    let styles = style_document(&document, &[]);

    assert_eq!(
        value(&styles, &document, "child", "color").as_deref(),
        Some("black")
    );
    assert_eq!(
        value(&styles, &document, "child", "margin-left").as_deref(),
        Some("5px")
    );
}

#[test]
fn applies_styles_to_custom_elements() {
    let document = parse_raw_html("<my-widget id='custom'></my-widget>".into());
    let sheet = parse_stylesheet("my-widget { color: red; }");
    let styles = style_document(&document, &[sheet]);

    assert_eq!(
        value(&styles, &document, "custom", "color").as_deref(),
        Some("red")
    );
}

#[test]
fn computed_styles_expose_declarations_and_parsed_colors() {
    let document = parse_raw_html("<p id='target'>Hello</p>".into());
    let sheet = parse_stylesheet("p { color: #ff0000; }");
    let styles = style_document(&document, &[sheet]);
    let node = document
        .get_element_by_id("target")
        .expect("target element");

    assert!(
        styles
            .declarations_for(node)
            .unwrap()
            .contains(&Declaration {
                property: "color".into(),
                value: "#ff0000".into(),
            })
    );
    assert_eq!(styles.color_for(node, "color"), css::parse_color("red"));
}

#[test]
fn viewport_context_applies_media_queries_and_viewport_font_sizes() {
    let document = parse_raw_html("<p id='target'>Hello</p>".into());
    let sheet = parse_stylesheet("@media (min-width: 600px) { p { font-size: 2vw; } }");
    let styles = style_document_with_context(
        &document,
        &[sheet],
        StyleContext {
            viewport_width: 800.0,
            viewport_height: 600.0,
            screen: true,
        },
    );

    assert_eq!(
        value(&styles, &document, "target", "font-size").as_deref(),
        Some("16px")
    );
}