use css::{parse_stylesheet, style_document_at};
use html::parse_raw_html;
use std::time::Duration;

#[test]
fn parses_keyframes_and_animation_rules() {
    let sheet = parse_stylesheet(
        "@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .box { animation: fade 1s linear; }",
    );

    let keyframes = sheet
        .at_rules
        .iter()
        .find(|rule| rule.name == "keyframes")
        .expect("keyframes rule");
    assert_eq!(keyframes.keyframes.len(), 2);
}

#[test]
fn samples_a_public_animation_through_the_cascade() {
    let document = parse_raw_html("<div id='target' class='box'></div>".into());
    let sheet = parse_stylesheet(
        "@keyframes fade { from { opacity: 0; } to { opacity: 1; } } .box { animation: fade 1s linear; }",
    );
    let styles = style_document_at(&document, &[sheet], Duration::from_millis(500));
    let node = document
        .get_element_by_id("target")
        .expect("target element");

    assert_eq!(styles.value_for(node, "opacity"), Some("0.5"));
}
