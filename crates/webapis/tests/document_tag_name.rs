//! Document tag lookup through the script bindings and mutable DOM.

use js::{Realm, Value};
use webapis::{Document, register_document};

fn realm(markup: &str) -> Realm {
    let mut realm = Realm::new();
    register_document(
        &mut realm,
        Document::from_html_document(&html::parse_raw_html(markup.to_owned())),
    )
    .unwrap();
    realm
}

#[test]
fn document_lookup_includes_root_and_preserves_tree_order_and_identity() {
    let realm = realm(
        "<html id='root'><body><p id='a'></p><section><p id='b' name='second'></p></section></body></html>",
    );
    for source in [
        "document.getElementsByTagName('HTML')[0] === document.getElementById('root')",
        "document.getElementsByTagName('P').length === 2",
        "document.getElementsByTagName('p')[0] === document.getElementById('a')",
        "document.getElementsByTagName('p').item(1) === document.getElementById('b')",
        "document.getElementsByTagName('p').namedItem('second') === document.getElementById('b')",
        "document.getElementsByTagName('p').item(9) === null",
        "document.getElementsByTagName('missing').length === 0",
        "document.getElementsByTagName('*').length === 5",
        "document.getElementsByTagName(null).length === 0",
        "var caught = false; try { document.getElementsByTagName(); } catch (e) { caught = e.name === 'TypeError'; } caught",
    ] {
        assert_eq!(
            realm.evaluate_script(source).unwrap(),
            Value::Boolean(true),
            "{source}"
        );
    }
}

#[test]
fn snapshots_stay_fixed_and_new_queries_see_mutations() {
    let realm = realm("<div id='root'><p></p></div>");
    assert_eq!(realm.evaluate_script(
        "var original = document.getElementsByTagName('p'); document.getElementById('root').appendChild(document.createElement('p')); original.length === 1 && document.getElementsByTagName('p').length === 2"
    ).unwrap(), Value::Boolean(true));
}

#[test]
fn foreign_names_remain_case_sensitive() {
    // The parser lowercases foreign names; construct a mixed-case SVG node
    // directly to exercise the binding's namespace rules independently.
    let mut source = html::HTMLDocument::new();
    let foreign = source.push_node(html::NodeKind::Element(html::ElementData::new(
        "foreignObject".into(),
        html::Namespace::Svg,
        vec![html::Attribute::new("id".into(), "foreign".into())],
    )));
    source.append_child(*source.get_root(), foreign);
    let mut realm = Realm::new();
    register_document(&mut realm, Document::from_html_document(&source)).unwrap();
    assert_eq!(realm.evaluate_script(
        "document.getElementsByTagName('foreignObject')[0] === document.getElementById('foreign') && document.getElementsByTagName('foreignobject').length === 0"
    ).unwrap(), Value::Boolean(true));
}
