//! Public-contract tests for `html`.
//!
//! Ignored tests describe behavior the HTML team still needs to implement.
//! Run them with `cargo test -p html -- --ignored` to see the backlog.

use common::ids::NodeId;
use html::{Query, Token, parse, parse_raw_html, tokenize};

const SIMPLE_HTML: &str = include_str!("../../../tests/html/simple.html");

#[test]
fn parses_simple_html_fixture_into_a_queryable_document() {
    let document = parse_raw_html(SIMPLE_HTML.to_owned());

    assert_eq!(document.doc_type, "html");
    assert_eq!(document.query("html").len(), 1);
    assert_eq!(document.query("meta").len(), 2);

    let viewport = document
        .query("meta")
        .into_iter()
        .find(|element| element.attributes.get("name") == Some(&"viewport".to_owned()))
        .expect("viewport meta element");
    assert_eq!(
        viewport.attributes.get("content"),
        Some(&"width=device-width, initial-scale=1.0".to_owned())
    );

    let head = document.query("head").pop().expect("head element");
    let html = document.query("html").pop().expect("html element");
    assert_eq!(head.parent, NodeId::new(0));
    assert_eq!(html.children, vec![NodeId::new(1), NodeId::new(5)]);
}

#[test]
#[ignore = "TODO(html): tokenizer not implemented"]
fn tokenizes_a_simple_element() {
    let tokens = tokenize("<p>hi</p>");
    assert_eq!(
        tokens,
        vec![
            Token::StartTag("p".into()),
            Token::Text("hi".into()),
            Token::EndTag("p".into()),
        ]
    );
}

#[test]
#[ignore = "TODO(html): tree builder not implemented"]
fn parses_nested_elements_with_parents() {
    let dom = parse("<html><body></body></html>");
    let body = dom
        .nodes
        .iter()
        .find(|n| n.name == "body")
        .expect("body node");
    let parent = body.parent.expect("body has a parent");
    assert_eq!(dom.nodes[parent.index() as usize].name, "html");
}

#[test]
#[ignore = "TODO(html): error recovery not implemented"]
fn malformed_input_does_not_panic() {
    // Real-world HTML is often unclosed; the parser must recover, not panic.
    let dom = parse("<div><p>unclosed");
    assert!(!dom.nodes.is_empty());
}
