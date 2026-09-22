//! Public-contract tests for `html`.
//!
//! Ignored tests describe behavior the HTML team still needs to implement.
//! Run them with `cargo test -p html -- --ignored` to see the backlog.

use html::{Token, parse, tokenize};

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
