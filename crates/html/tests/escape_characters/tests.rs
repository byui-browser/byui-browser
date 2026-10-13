use html::{HTMLDocument, NodeId, NodeKind, Query, parse_raw_html};
#[path = "../../src/html5_entities.rs"]
mod entity_table;

/// HELPER Functions for the tests below, this isn't supposed to be actual API production code
/// Returns the first element node with the given tag name.
///
/// The lookup scans the document's arena order. It panics when no matching
/// element exists; callers that need optional lookup should use `query`.
fn element_id(document: &HTMLDocument, name: &str) -> NodeId {
    document
        .nodes
        .iter()
        .position(|node| matches!(&node.kind, NodeKind::Element(element) if element.name == name))
        .map(NodeId)
        .unwrap_or_else(|| panic!("missing <{name}> element"))
}

/// Returns an element's named attribute value, if present.
///
/// The returned string is borrowed from `document` and is valid for the same
/// lifetime as the document reference. Panics if `element_name` is absent.
fn attribute_value<'a>(
    document: &'a HTMLDocument,
    element_name: &str,
    name: &str,
) -> Option<&'a str> {
    let id = element_id(document, element_name);
    let NodeKind::Element(element) = &document.nodes[id.index()].kind else {
        unreachable!("element lookup returns an element")
    };
    element
        .attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| attribute.value.as_str())
}

/// Collects the text content of the first `<p>` element in document order.
///
/// Panics when the document has no paragraph element.
fn paragraph_text(document: &HTMLDocument) -> String {
    document.text_content(element_id(document, "p"))
}

/// Parses `source` as paragraph text and returns its decoded text content.
///
/// Character references are decoded once, and decoded markup characters
/// remain text. This helper uses the HTML parser's current partial tree builder.
fn decode_text(source: &str) -> String {
    paragraph_text(&parse_raw_html(format!("<p>{source}</p>")))
}

#[test]
fn all_html5_named_references_decode() {
    for (name, expected) in entity_table::NAMED_REFERENCES {
        let decoded = html::parse_raw_html(format!("<p>&{name}</p>"));
        decoded.query("p").pop().expect("paragraph");
        assert_eq!(paragraph_text(&decoded), *expected, "reference &{name}");
    }
}

#[test]
fn text_references_render_as_text_without_creating_markup() {
    let variants = [
        ("&lt;script&gt;", "<script>"),
        ("&lt;img&gt;", "<img>"),
        ("&lt;div&gt;", "<div>"),
        ("&lt;p&gt;", "<p>"),
        ("&lt;iframe&gt;", "<iframe>"),
        ("&lt;svg&gt;", "<svg>"),
        ("&lt;style&gt;", "<style>"),
        ("&lt;table&gt;", "<table>"),
        ("&lt;form&gt;", "<form>"),
        ("&lt;button&gt;", "<button>"),
    ];
    for (source, expected) in variants {
        let document = parse_raw_html(format!("<main>{source}</main>"));
        document.query("main").pop().unwrap();
        assert_eq!(
            document.text_content(element_id(&document, "main")),
            expected
        );
        assert_eq!(document.query("script").len(), 0);
    }
}

#[test]
fn ten_numeric_decimal_references_decode() {
    let variants = [
        ("&#38;", "&"),
        ("&#60;", "<"),
        ("&#62;", ">"),
        ("&#34;", "\""),
        ("&#39;", "'"),
        ("&#169;", "©"),
        ("&#174;", "®"),
        ("&#8364;", "€"),
        ("&#8212;", "—"),
        ("&#9731;", "☃"),
    ];
    for (source, expected) in variants {
        assert_eq!(decode_text(source), expected);
    }
}

#[test]
fn ten_numeric_hexadecimal_references_decode() {
    let variants = [
        ("&#x26;", "&"),
        ("&#x3C;", "<"),
        ("&#x3E;", ">"),
        ("&#x22;", "\""),
        ("&#x27;", "'"),
        ("&#xA9;", "©"),
        ("&#xAE;", "®"),
        ("&#x20AC;", "€"),
        ("&#x2014;", "—"),
        ("&#x2603;", "☃"),
    ];
    for (source, expected) in variants {
        assert_eq!(decode_text(source), expected);
    }
}

#[test]
fn numeric_references_handle_invalid_and_c1_code_points() {
    let variants = [
        ("&#0;", "�"),
        ("&#xD800;", "�"),
        ("&#xDFFF;", "�"),
        ("&#x110000;", "�"),
        ("&#99999999999999999999;", "�"),
        ("&#xFFFFFFFFFFFFFFFF;", "�"),
        ("&#128;", "€"),
        ("&#x80;", "€"),
        ("&#130;", "‚"),
        ("&#x9F;", "Ÿ"),
    ];
    for (source, expected) in variants {
        assert_eq!(decode_text(source), expected);
    }
}

#[test]
fn named_references_decode_across_symbol_categories() {
    let variants = [
        ("&amp;", "&"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&copy;", "©"),
        ("&reg;", "®"),
        ("&euro;", "€"),
        ("&trade;", "™"),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&hearts;", "♥"),
    ];
    for (source, expected) in variants {
        assert_eq!(decode_text(source), expected);
    }
}

#[test]
fn references_decode_inside_double_quoted_attributes() {
    let document = parse_raw_html("<a title=\"A &amp; B &copy;\">x</a>".into());
    document.query("a").pop().unwrap();
    assert_eq!(attribute_value(&document, "a", "title"), Some("A & B ©"));
}

#[test]
fn references_decode_inside_single_quoted_attributes() {
    let document = parse_raw_html("<a title='A &lt; B &gt;'>x</a>".into());
    document.query("a").pop().unwrap();
    assert_eq!(attribute_value(&document, "a", "title"), Some("A < B >"));
}

#[test]
fn ambiguous_semicolonless_attribute_references_are_preserved() {
    let document = parse_raw_html("<a title='&notit; &ampx &copy=ok'>x</a>".into());
    document.query("a").pop().unwrap();
    assert_eq!(
        attribute_value(&document, "a", "title"),
        Some("&notit; &ampx &copy=ok")
    );
}

#[test]
fn decoded_less_than_stays_a_text_node() {
    let document = parse_raw_html("<p>&lt;h1&gt;hello&lt;/h1&gt;</p>".into());
    assert_eq!(document.query("h1").len(), 0);
    document.query("p").pop().unwrap();
    let id = element_id(&document, "p");
    assert_eq!(document.text_content(id), "<h1>hello</h1>");
    assert!(
        document
            .nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Text(text) if text == "<h1>hello</h1>"))
    );
}

#[test]
fn unknown_and_incomplete_references_are_left_intact() {
    for source in ["&unknown;", "&#", "&#x", "plain & text"] {
        assert_eq!(decode_text(source), source);
    }
    assert_eq!(decode_text("&amp"), "&");
}

#[test]
fn decoded_ampersands_are_not_decoded_twice() {
    assert_eq!(decode_text("&amp;lt;"), "&lt;");
}
