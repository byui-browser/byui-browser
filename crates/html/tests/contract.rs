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
fn inner_html_serializes_markup_and_escapes_text_and_attributes() {
    let mut document = HTMLDocument::new();
    let paragraph = document.push_node(
        NodeKind::Element(html::ElementData {
            name: "p".into(),
            namespace: Namespace::Html,
            attributes: vec![Attribute {
                name: "title".into(),
                value: "A & \"B\"".into(),
            }],
        }),
        None,
    );
    let text = document.push_node(NodeKind::Text("<hello & goodbye>".into()), None);
    document.append_child(document.root, paragraph);
    document.append_child(paragraph, text);

    assert_eq!(
        document.inner_html(),
        "<p title=\"A &amp; &quot;B&quot;\">&lt;hello &amp; goodbye></p>"
    );
}

#[test]
fn create_element_follows_html_document_rules() {
    let mut document = HTMLDocument::new();

    let element = document.create_element("DiV-Card").expect("valid name");
    let node = document.node(element).expect("created node");

    assert_eq!(node.parent, None);
    assert_eq!(node.children, Vec::new());
    assert!(matches!(
        &node.kind,
        NodeKind::Element(ElementData {
            name,
            namespace: Namespace::Html,
            attributes,
        }) if name == "div-card" && attributes.is_empty()
    ));
}

#[test]
fn create_element_rejects_invalid_names() {
    let mut document = HTMLDocument::new();

    for name in ["", "1div", "div name", "div/name", "div>"] {
        assert_eq!(
            document.create_element(name),
            Err(html::DomError::InvalidCharacter),
            "expected {name:?} to be rejected"
        );
    }
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
            .expect("body text")
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
fn get_element_by_id_returns_the_first_matching_descendant_in_tree_order() {
    let document = parse_raw_html(
        "<div id='duplicate'><span id='target'></span></div><p id='target'></p>".to_owned(),
    );

    let first_target = document
        .get_element_by_id("target")
        .expect("target element");
    assert_eq!(document.nodes[first_target.0].parent, Some(html::NodeId(1)));
}

#[test]
fn get_element_by_id_matches_exactly_and_ignores_detached_nodes() {
    let mut document = parse_raw_html("<div id='target'></div>".to_owned());
    let detached = document.push_node(
        NodeKind::Element(ElementData {
            name: "span".into(),
            namespace: Namespace::Html,
            attributes: vec![Attribute {
                name: "id".into(),
                value: "detached".into(),
            }],
        }),
        None,
    );

    assert_eq!(document.get_element_by_id("TARGET"), None);
    assert_eq!(document.get_element_by_id("detached"), None);
    assert_eq!(document.get_element_by_id("target"), Some(html::NodeId(1)));
    assert_eq!(document.node(detached).unwrap().parent, None);
}

#[test]
fn get_element_by_id_accepts_an_empty_id() {
    let document = parse_raw_html("<div id=''></div>".to_owned());

    assert_eq!(document.get_element_by_id(""), Some(html::NodeId(1)));
}

#[test]
fn class_list_exposes_the_class_attribute_as_an_ordered_set() {
    let mut document = parse_raw_html("<div class='  card card featured\t'></div>".to_owned());
    let element = html::NodeId(1);
    let mut class_list = document.class_list(element).expect("element class list");

    assert_eq!(class_list.length(), 2);
    assert_eq!(class_list.item(0), Some("card".to_owned()));
    assert_eq!(class_list.item(2), None);
    assert!(class_list.contains("featured"));
    assert_eq!(class_list.value(), "  card card featured\t");

    class_list.add(&["selected", "card"]).unwrap();
    assert_eq!(class_list.value(), "card featured selected");
    class_list.remove(&["featured"]).unwrap();
    assert_eq!(class_list.value(), "card selected");
}

#[test]
fn class_list_mutations_update_the_element_attribute() {
    let mut document = parse_raw_html("<div class='card'></div>".to_owned());
    let element = html::NodeId(1);

    {
        let mut class_list = document.class_list(element).unwrap();
        assert_eq!(class_list.toggle("active", None), Ok(true));
        assert_eq!(class_list.toggle("active", Some(false)), Ok(false));
        assert_eq!(class_list.replace("card", "panel"), Ok(true));
        class_list.set_value("panel  wide wide").unwrap();
    }

    let NodeKind::Element(element) = &document.node(html::NodeId(1)).unwrap().kind else {
        panic!("expected element");
    };
    assert_eq!(
        element
            .attributes
            .iter()
            .find(|attribute| attribute.name == "class")
            .map(|attribute| attribute.value.as_str()),
        Some("panel  wide wide")
    );
}

#[test]
fn class_list_validates_all_tokens_before_mutating() {
    let mut document = parse_raw_html("<div class='card'></div>".to_owned());
    let mut class_list = document.class_list(html::NodeId(1)).unwrap();

    assert_eq!(
        class_list.add(&["valid", "has space"]),
        Err(html::DomError::InvalidToken)
    );
    assert_eq!(class_list.value(), "card");
    assert_eq!(class_list.remove(&[""]), Err(html::DomError::Syntax));
    assert_eq!(class_list.supports("card"), Err(html::DomError::Type));
}

#[test]
fn remove_detaches_a_node_but_preserves_its_subtree() {
    let mut document =
        parse_raw_html("<section><span id='target'>text</span></section><p></p>".to_owned());
    let section = html::NodeId(1);
    let span = document.get_element_by_id("target").expect("target");
    let text = document
        .nodes
        .iter()
        .position(|node| matches!(&node.kind, NodeKind::Text(value) if value == "text"))
        .map(html::NodeId)
        .expect("text node");

    document.remove(section);

    assert_eq!(document.node(section).unwrap().parent, None);
    assert_eq!(document.node(span).unwrap().parent, Some(section));
    assert_eq!(document.node(text).unwrap().parent, Some(span));
    assert_eq!(document.get_element_by_id("target"), None);
}

#[test]
fn remove_of_detached_or_root_nodes_is_a_no_op() {
    let mut document = parse_raw_html("<div></div>".to_owned());
    let detached = document.push_node(NodeKind::Comment("detached".into()), None);
    let children_before = document.nodes[document.root.0].children.clone();

    document.remove(detached);
    document.remove(document.root);

    assert_eq!(document.node(detached).unwrap().parent, None);
    assert_eq!(document.nodes[document.root.0].children, children_before);
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
    assert_eq!(document.text_content(text_nodes[0].0), Some("Hello".into()));

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
    assert_eq!(text.text_content(text.root), None);
}

#[test]
fn text_content_matches_dom_node_type_rules() {
    let mut document = HTMLDocument::new();
    let element = document.push_node(
        NodeKind::Element(ElementData {
            name: "section".into(),
            namespace: Namespace::Html,
            attributes: Vec::new(),
        }),
        None,
    );
    let first_text = document.push_node(NodeKind::Text("before".into()), None);
    let comment = document.push_node(NodeKind::Comment("ignored by elements".into()), None);
    let second_text = document.push_node(NodeKind::Text("after".into()), None);
    let doctype = document.push_node(
        NodeKind::Doctype {
            name: Some("html".into()),
            public_id: None,
            system_id: None,
        },
        None,
    );
    document.append_child(document.root, element);
    document.append_child(element, first_text);
    document.append_child(element, comment);
    document.append_child(element, second_text);

    assert_eq!(document.text_content(element), Some("beforeafter".into()));
    assert_eq!(document.text_content(first_text), Some("before".into()));
    assert_eq!(
        document.text_content(comment),
        Some("ignored by elements".into())
    );
    assert_eq!(document.text_content(document.root), None);
    assert_eq!(document.text_content(doctype), None);
    assert_eq!(document.text_content(html::NodeId(999)), None);
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
    let first_id = document
        .nodes
        .iter()
        .position(|node| node == &paragraphs[0])
        .map(html::NodeId)
        .expect("first paragraph id");
    let second_id = document
        .nodes
        .iter()
        .position(|node| node == &paragraphs[1])
        .map(html::NodeId)
        .expect("second paragraph id");
    assert_eq!(document.text_content(first_id), Some("first".into()));
    assert_eq!(document.text_content(second_id), Some("second".into()));
}

#[test]
fn preserves_void_element_siblings_and_does_not_push_void_nodes() {
    let document = parse_raw_html("<p>before<br>after<img src=x>tail</p>".to_owned());
    let paragraph = document.query("p").pop().expect("paragraph");
    assert_eq!(
        document.text_content(html::NodeId(paragraph.id.index() as usize)),
        "beforeaftertail"
    let paragraph_id = document
        .nodes
        .iter()
        .position(|node| node == &paragraph)
        .map(html::NodeId)
        .expect("paragraph id");
    assert_eq!(
        document.text_content(paragraph_id),
        Some("beforeaftertail".into())
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
    assert_eq!(incomplete.text_content(incomplete.root), None);

    let attributes = parse_raw_html("<input value='unfinished>".to_owned());
    let input = attributes.query("input").pop().expect("input");
    assert_eq!(
        input.attributes.get("value"),
        Some(&"unfinished".to_owned())
    );
}

#[test]
fn extracts_inline_script_sources_in_document_order() {
    let document = parse_raw_html(
        "<script>print()</script><div><script>fetch('https://example.com')</script></div>"
            .to_owned(),
    );

    assert_eq!(
        document.script_sources(),
        ["print()", "fetch('https://example.com')"]
    );
}
