use html::{
    Attribute, ElementData, HtmlDocument, Namespace, NodeKind, Query, Token, parse, parse_raw_html,
    tokenize,
};

const SIMPLE_HTML: &str = include_str!("../../../tests/html/simple.html");
const VALID_HTML: &str = include_str!("../../../tests/html/valid.html");

#[test]
fn arena_document_starts_with_a_document_root() {
    let document = HtmlDocument::new();
    assert_eq!(document.root.0, 0);
    assert!(matches!(
        document.nodes[document.root.0].kind,
        NodeKind::Document
    ));
}

#[test]
fn arena_nodes_are_connected_with_stable_ids_and_spans() {
    let mut document = HtmlDocument::new();
    let paragraph = document.push_node(
        NodeKind::Element(ElementData {
            name: "p".into(),
            namespace: Namespace::Html,
            attributes: vec![Attribute {
                name: "class".into(),
                value: "intro".into(),
            }],
        }),
        None,
    );
    let text = document.push_node(
        NodeKind::Text("Hello".into()),
        Some(html::SourceSpan { start: 0, end: 5 }),
    );
    document.append_child(document.root, paragraph);
    document.append_child(paragraph, text);
    assert_eq!(document.nodes[paragraph.0].parent, Some(document.root));
    assert_eq!(document.nodes[paragraph.0].children, vec![text]);
    assert_eq!(
        document.nodes[text.0].span,
        Some(html::SourceSpan { start: 0, end: 5 })
    );
}

#[test]
fn parses_simple_fixture_into_typed_nodes_and_queries() {
    let document = parse_raw_html(SIMPLE_HTML.to_owned());
    assert_eq!(document.query("html").len(), 1);
    assert_eq!(document.query("meta").len(), 2);
    let viewport = document
        .query("meta")
        .into_iter()
        .find(|element| element.attributes.get("name") == Some(&"viewport".to_owned()))
        .expect("viewport");
    assert_eq!(
        viewport.attributes.get("content"),
        Some(&"width=device-width, initial-scale=1.0".to_owned())
    );
    assert!(document.nodes.iter().any(
        |node| matches!(&node.kind, NodeKind::Doctype { name: Some(name), .. } if name == "html")
    ));
}

#[test]
fn tokenizes_tags_text_doctypes_and_comments() {
    assert_eq!(
        tokenize("<!doctype html><!-- note --><p>hi</p>"),
        vec![
            Token::Doctype("html".into()),
            Token::Comment(" note ".into()),
            Token::StartTag("p".into()),
            Token::Text("hi".into()),
            Token::EndTag("p".into()),
        ]
    );
}

#[test]
fn builds_legacy_tree_without_pushing_void_elements() {
    let dom = parse("<p>before<br>after</p><p>next</p>");
    let paragraphs = dom
        .nodes
        .iter()
        .filter(|node| node.name == "p")
        .collect::<Vec<_>>();
    assert_eq!(paragraphs.len(), 2);
    assert_eq!(paragraphs[1].parent, None);
    assert_eq!(
        dom.nodes.iter().filter(|node| node.name == "#text").count(),
        3
    );
}

#[test]
fn malformed_legacy_input_does_not_panic() {
    assert!(!parse("<div><p>unclosed").nodes.is_empty());
}

#[test]
fn valid_fixture_has_expected_arena_shape_and_text() {
    assert_eq!(VALID_HTML.lines().count(), 100);
    let document = parse_raw_html(VALID_HTML.to_owned());
    let body_id = document.get_element_by_id("page").expect("body id");
    let html_id = document.nodes[body_id.0].parent.expect("html parent");
    assert!(
        document
            .text_content(body_id)
            .contains("Building a Small HTML Tree")
    );
    assert!(
        document.nodes.iter().any(
            |node| matches!(&node.kind, NodeKind::Element(element) if element.name == "footer")
        )
    );
    assert!(
        matches!(&document.nodes[html_id.0].kind, NodeKind::Element(element) if element.name == "html")
    );
}

#[test]
fn parses_attributes_and_boolean_attributes_as_ordered_values() {
    let document = parse_raw_html("<input disabled class='field' data-count=3>".to_owned());
    let input = document.query("input").pop().expect("input");
    assert_eq!(input.attributes.get("disabled"), Some(&String::new()));
    assert_eq!(input.attributes.get("class"), Some(&"field".to_owned()));
    assert_eq!(input.attributes.get("data-count"), Some(&"3".to_owned()));
}

#[test]
fn parse_5_nested_children_finds_the_text_node() {
    let document =
        parse_raw_html("<div><div><div><div><div>Hello</div></div></div></div></div>".to_owned());

    let text_nodes = document
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| match &node.kind {
            NodeKind::Text(text) => Some((html::NodeId(index), text.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(text_nodes, vec![(html::NodeId(6), "Hello")]);
    assert_eq!(document.text_content(text_nodes[0].0), "Hello");

    let mut current = text_nodes[0].0;
    for _ in 0..5 {
        current = document.nodes[current.0].parent.expect("nested parent");
        assert!(matches!(
            document.nodes[current.0].kind,
            NodeKind::Element(_)
        ));
    }
    assert_eq!(document.nodes[current.0].parent, Some(document.root));
}

#[test]
fn handles_empty_input_and_text_without_markup() {
    let empty = parse_raw_html(String::new());
    assert_eq!(empty.nodes.len(), 1);
    assert!(matches!(empty.nodes[empty.root.0].kind, NodeKind::Document));

    let text = parse_raw_html("plain text < and > signs".to_owned());
    assert_eq!(text.text_content(text.root), "plain text < and > signs");
}

#[test]
fn handles_unclosed_tags_and_mismatched_closing_tags() {
    let document = parse_raw_html("<div><p>first</div><p>second".to_owned());
    let paragraphs = document.query("p");
    assert_eq!(paragraphs.len(), 2);
    assert_eq!(
        document.text_content(html::NodeId(paragraphs[0].id.index() as usize)),
        "first"
    );
    assert_eq!(
        document.text_content(html::NodeId(paragraphs[1].id.index() as usize)),
        "second"
    );
}

#[test]
fn preserves_void_element_siblings_and_does_not_push_void_nodes() {
    let document = parse_raw_html("<p>before<br>after<img src=x>tail</p>".to_owned());
    let paragraph = document.query("p").pop().expect("paragraph");
    assert_eq!(
        document.text_content(html::NodeId(paragraph.id.index() as usize)),
        "beforeaftertail"
    );
    for tag in ["br", "img"] {
        let node_id = paragraph
            .children
            .iter()
            .copied()
            .find(|id| {
                matches!(
                    &document.nodes[id.index() as usize].kind,
                    NodeKind::Element(element) if element.name == tag
                )
            })
            .expect("void element child");
        assert!(document.nodes[node_id.index() as usize].children.is_empty());
    }
}

#[test]
fn keeps_incomplete_tag_text_and_unterminated_attribute_values() {
    let incomplete = parse_raw_html("hello <p".to_owned());
    assert_eq!(incomplete.text_content(incomplete.root), "hello <p");

    let attributes = parse_raw_html("<input value='unfinished>".to_owned());
    let input = attributes.query("input").pop().expect("input");
    assert_eq!(
        input.attributes.get("value"),
        Some(&"unfinished".to_owned())
    );
}
