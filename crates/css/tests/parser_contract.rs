//! Public integration tests for CSS stylesheet parsing.

use css::{parse_stylesheet, parse_stylesheet_with_errors};

#[test]
fn parses_multiple_style_rules() {
    let sheet = parse_stylesheet("body { color: red; } p { margin: 0; }");

    assert_eq!(sheet.rules.len(), 2);
    assert_eq!(sheet.rules[0].selector.0, "body");
    assert_eq!(sheet.rules[1].selector.0, "p");
}

#[test]
fn parses_multiple_declarations_in_source_order() {
    let sheet = parse_stylesheet("p { color: red; background-color: blue; margin: 10px; }");

    let declarations = &sheet.rules[0].declarations;

    assert_eq!(declarations.len(), 3);
    assert_eq!(declarations[0].property, "color");
    assert_eq!(declarations[0].value, "red");
    assert_eq!(declarations[1].property, "background-color");
    assert_eq!(declarations[1].value, "blue");
    assert_eq!(declarations[2].property, "margin");
    assert_eq!(declarations[2].value, "10px");
}

#[test]
fn preserves_unknown_and_custom_properties() {
    let sheet =
        parse_stylesheet("custom-element { future-property: future(value); --custom-value: 42; }");

    let declarations = &sheet.rules[0].declarations;

    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0].property, "future-property");
    assert_eq!(declarations[0].value, "future(value)");
    assert_eq!(declarations[1].property, "--custom-value");
    assert_eq!(declarations[1].value, "42");
}

#[test]
fn preserves_original_stylesheet_source() {
    let source = "/* stylesheet */\narticle { color: red; }";
    let sheet = parse_stylesheet(source);

    assert_eq!(sheet.source, source);
}

#[test]
fn parses_statement_and_block_at_rules() {
    let sheet = parse_stylesheet("@import url(\"theme.css\"); @media screen { p { color: red; } }");

    assert!(sheet.at_rules.iter().any(|rule| rule.name == "import"));
    assert!(sheet.at_rules.iter().any(|rule| rule.name == "media"));

    let media = sheet
        .at_rules
        .iter()
        .find(|rule| rule.name == "media")
        .expect("media at-rule");

    assert_eq!(media.rules.len(), 1);
    assert_eq!(media.rules[0].selector.0, "p");
}

#[test]
fn reports_errors_and_recovers_valid_neighboring_rules() {
    let source = "p { color: red; } broken { color: ; } h1 { color: blue; }";
    let (sheet, errors) = parse_stylesheet_with_errors(source);

    assert!(!errors.is_empty());
    assert_eq!(sheet.rules.len(), 3);
    assert_eq!(sheet.rules[0].selector.0, "p");
    assert_eq!(sheet.rules[2].selector.0, "h1");
}

#[test]
fn parses_the_common_stylesheet_rule_families_without_losing_structure() {
    let source = r#"
        @charset "UTF-8";
        @namespace svg url("http://www.w3.org/2000/svg");
        @import url("theme.css") screen;
        @layer reset, theme;
        @media screen { body { color: red; } }
        @supports (display: grid) { .grid { display: grid; } }
        @keyframes fade { from { opacity: 0; } to { opacity: 1; } }
        @font-face { font-family: Demo; src: url(demo.woff2); }
        @page :first { margin: 1cm; @top-left { content: "x"; } }
        @property --accent { syntax: "<color>"; inherits: true; initial-value: red; }
        @unknown future(value) { preserved: yes; }
    "#;

    let (sheet, errors) = parse_stylesheet_with_errors(source);
    assert!(errors.is_empty(), "unexpected parse errors: {errors:?}");
    assert_eq!(sheet.at_rules.len(), 11);

    let media = sheet
        .at_rules
        .iter()
        .find(|rule| rule.name == "media")
        .unwrap();
    assert_eq!(media.rules[0].selector.0, "body");

    let keyframes = sheet
        .at_rules
        .iter()
        .find(|rule| rule.name == "keyframes")
        .unwrap();
    assert_eq!(keyframes.rules.len(), 2);
    assert_eq!(keyframes.rules[0].declarations[0].property, "opacity");

    let unknown = sheet
        .at_rules
        .iter()
        .find(|rule| rule.name == "unknown")
        .unwrap();
    assert_eq!(unknown.block.as_deref(), Some("preserved: yes;"));
}

#[test]
fn preserves_nested_component_values_in_declarations() {
    let sheet = parse_stylesheet(
        r#"article {
            color: color-mix(in srgb, var(--brand, rgb(1 2 3)) 40%, white);
            background: url("semi;colon.png") no-repeat;
            content: "{ not a block }";
            --raw: { nested: [one, two]; };
        }"#,
    );

    assert!(sheet.errors.is_empty());
    let declarations = &sheet.rules[0].declarations;
    assert_eq!(declarations.len(), 4);
    assert!(declarations[0].value.contains("color-mix"));
    assert_eq!(declarations[1].value, r#"url("semi;colon.png") no-repeat"#);
    assert_eq!(declarations[2].value, r#""{ not a block }""#);
    assert_eq!(declarations[3].property, "--raw");
}