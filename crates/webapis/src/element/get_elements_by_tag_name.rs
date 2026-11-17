//! `Element.getElementsByTagName()` on script element wrappers.

use std::sync::{Arc, Mutex};

use html::{HTMLDocument, Namespace, NodeID, NodeKind};
use js::{JsError, Object, Value};

use crate::document::Document;
use crate::element::{Element, wrap_element};

/// Installs `Element.getElementsByTagName()` on the wrapper `object` for `element`.
///
/// The method implements the DOM Standard's "list of elements with qualified
/// name": it returns the element's descendants (never the element itself) in
/// tree order whose qualified name matches the argument. `"*"` matches every
/// descendant element. HTML-namespace elements are compared against the
/// ASCII-lowercased argument; SVG and MathML elements are compared against the
/// argument as given. The argument is converted with `ToString`, and calling
/// the method without an argument throws a `TypeError`.
///
/// Each matching element is returned as its shared wrapper, so results are
/// identical (`===`) to wrappers from `getElementById()` and friends. The
/// collection also exposes `item(index)` and `namedItem(name)`.
///
/// Limitation: the DOM returns a live `HTMLCollection`; this returns a
/// snapshot array-like taken when the method is called, so later tree changes
/// are not reflected in it.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = document.clone();
    object.set_method(
        "getElementsByTagName",
        Arc::new(move |arguments| {
            let Some(argument) = arguments.first() else {
                return Err(type_error(
                    "Element.getElementsByTagName() requires 1 argument, but 0 were present",
                ));
            };
            let qualified_name = argument.to_string();
            // Collect IDs under the lock, then release it before wrapping:
            // `wrap_element` locks the document itself.
            let matches = {
                let document = document.lock().unwrap();
                document
                    .html_document()
                    .map(|source| matching_descendants(source, node, &qualified_name))
                    .unwrap_or_default()
            };
            log::debug!(
                "Element.getElementsByTagName({qualified_name:?}) matched {} elements",
                matches.len()
            );
            let mut entries = Vec::with_capacity(matches.len());
            for (id, names) in matches {
                entries.push((wrap_element(&document, id)?, names));
            }
            Ok(Value::Array(collection(entries)))
        }),
    );
}

/// Values of a matched element's `id` and `name` attributes, captured for
/// `HTMLCollection.namedItem()` while the document lock is held.
type ItemNames = (Option<String>, Option<String>);

/// Returns the descendants of `root` (excluding `root`) in tree order whose
/// qualified name matches `qualified_name`, with their `id` and `name`
/// attribute values.
fn matching_descendants(
    source: &HTMLDocument,
    root: NodeID,
    qualified_name: &str,
) -> Vec<(NodeID, ItemNames)> {
    let lowercase = qualified_name.to_ascii_lowercase();
    let mut matches = Vec::new();
    let mut stack: Vec<NodeID> = source
        .node(root)
        .map(|node| node.get_children().iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = source.node(id) else {
            continue;
        };
        if let NodeKind::Element(data) = node.get_kind() {
            let matched = qualified_name == "*"
                || match data.get_namespace() {
                    Namespace::Html => *data.get_name() == lowercase,
                    Namespace::Svg | Namespace::MathMl => *data.get_name() == qualified_name,
                };
            if matched {
                let attribute = |name: &str| {
                    data.get_attributes()
                        .iter()
                        .find(|attribute| attribute.get_name() == name)
                        .map(|attribute| attribute.get_value().clone())
                };
                let is_html = *data.get_namespace() == Namespace::Html;
                let name = if is_html { attribute("name") } else { None };
                matches.push((id, (attribute("id"), name)));
            }
        }
        stack.extend(node.get_children().iter().rev().copied());
    }
    matches
}

/// Builds the snapshot `HTMLCollection` array-like for `entries`.
fn collection(entries: Vec<(Value, ItemNames)>) -> Object {
    let collection = Object::new();
    for (index, (wrapper, _)) in entries.iter().enumerate() {
        collection.set(index.to_string(), wrapper.clone());
    }
    collection.set("length", Value::Number(entries.len() as f64));

    let wrappers: Vec<Value> = entries.iter().map(|(wrapper, _)| wrapper.clone()).collect();
    collection.set_method(
        "item",
        Arc::new(move |arguments| {
            let index = match arguments.first() {
                Some(Value::Number(number)) => *number,
                Some(other) => other.to_string().trim().parse::<f64>().unwrap_or(0.0),
                None => 0.0,
            };
            if !index.is_finite() || index < 0.0 {
                return Ok(Value::Null);
            }
            Ok(wrappers
                .get(index.trunc() as usize)
                .cloned()
                .unwrap_or(Value::Null))
        }),
    );

    collection.set_method(
        "namedItem",
        Arc::new(move |arguments| {
            let name = arguments.first().map(Value::to_string).unwrap_or_default();
            if name.is_empty() {
                return Ok(Value::Null);
            }
            let found = entries.iter().find(|(_, (id, name_attr))| {
                id.as_deref() == Some(name.as_str()) || name_attr.as_deref() == Some(name.as_str())
            });
            Ok(found.map_or(Value::Null, |(wrapper, _)| wrapper.clone()))
        }),
    );
    collection
}

/// Creates a script-catchable `TypeError` exception.
fn type_error(message: impl Into<String>) -> JsError {
    let exception = Object::new();
    exception.set("name", Value::String("TypeError".into()));
    exception.set("message", Value::String(message.into()));
    JsError::thrown(Value::Object(exception))
}

#[cfg(test)]
mod tests {
    use html::{Attribute, ElementData, HTMLDocument, Namespace, NodeKind, parse_raw_html};
    use js::{Realm, Value};

    use crate::document::{Document, register_document};

    fn realm_with(markup: &str) -> Realm {
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            Document::from_html_document(&parse_raw_html(markup.to_owned())),
        )
        .unwrap();
        realm
    }

    fn eval(realm: &mut Realm, script: &str) -> Value {
        realm
            .evaluate_script(script)
            .unwrap_or_else(|error| panic!("{script}: {error:?}"))
    }

    fn string(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    const TREE: &str = "<div id='root'><p id='a'></p><section id='s'><p id='b'></p><span id='c'></span></section></div>\
                        <p id='outside'></p>";

    #[test]
    fn get_elements_by_tag_name_matches_html_names_case_insensitively_in_tree_order() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "const list = document.getElementById('root').getElementsByTagName('P');\
                 list.length + ':' + list[0].getAttribute('id') + ',' + list[1].getAttribute('id')"
            ),
            string("2:a,b")
        );
    }

    #[test]
    fn get_elements_by_tag_name_star_returns_all_descendants_excluding_self() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "const all = document.getElementById('root').getElementsByTagName('*');\
                 let ids = '';\
                 for (let i = 0; i < all.length; i++) { ids += all[i].getAttribute('id') + ' '; }\
                 ids"
            ),
            string("a s b c ")
        );
    }

    #[test]
    fn get_elements_by_tag_name_excludes_self_and_elements_outside_subtree() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "const section = document.getElementById('s');\
                 const ps = section.getElementsByTagName('p');\
                 const sections = section.getElementsByTagName('section');\
                 ps.length + ':' + ps[0].getAttribute('id') + ':' + sections.length"
            ),
            string("1:b:0")
        );
    }

    #[test]
    fn get_elements_by_tag_name_returns_shared_wrappers() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "const list = document.getElementById('root').getElementsByTagName('span');\
                 list[0] === document.getElementById('c') && list.item(0) === list[0]\
                 && list.namedItem('c') === list[0] && list.item(5) === null\
                 && list.namedItem('nope') === null"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn get_elements_by_tag_name_returns_empty_collection_without_matches() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "const list = document.getElementById('root').getElementsByTagName('table');\
                 list.length + ':' + (list[0] === undefined) + ':' + (list.item(0) === null)"
            ),
            string("0:true:true")
        );
    }

    #[test]
    fn get_elements_by_tag_name_converts_argument_with_to_string() {
        let mut realm = realm_with("<div id='root'><null></null><p></p></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('root').getElementsByTagName(null).length"
            ),
            Value::Number(1.0)
        );
    }

    #[test]
    fn get_elements_by_tag_name_compares_svg_names_case_sensitively() {
        // The HTML parser lowercases tag names even inside SVG, so build
        // `<div id=root><svg><foreignObject/></svg></div>` by hand to keep
        // the mixed-case `foreignObject` name.
        let mut source = HTMLDocument::new();
        let element = |source: &mut HTMLDocument, name: &str, namespace, id: Option<&str>| {
            let attributes = id
                .map(|id| vec![Attribute::new("id".to_owned(), id.to_owned())])
                .unwrap_or_default();
            source.push_node(NodeKind::Element(ElementData::new(
                name.to_owned(),
                namespace,
                attributes,
            )))
        };
        let div = element(&mut source, "div", Namespace::Html, Some("root"));
        let svg = element(&mut source, "svg", Namespace::Svg, None);
        let foreign = element(&mut source, "foreignObject", Namespace::Svg, None);
        let root = source.root;
        source.append_child(root, div);
        source.append_child(div, svg);
        source.append_child(svg, foreign);
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&source)).unwrap();

        assert_eq!(
            eval(
                &mut realm,
                "const root = document.getElementById('root');\
                 root.getElementsByTagName('foreignObject').length + ':' +\
                 root.getElementsByTagName('foreignobject').length + ':' +\
                 root.getElementsByTagName('svg').length + ':' +\
                 root.getElementsByTagName('SVG').length"
            ),
            string("1:0:1:0")
        );
    }

    #[test]
    fn get_elements_by_tag_name_without_argument_throws_type_error() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "let name = 'none';\
                 try { document.getElementById('root').getElementsByTagName(); }\
                 catch (error) { name = error.name; }\
                 name"
            ),
            string("TypeError")
        );
    }
}
