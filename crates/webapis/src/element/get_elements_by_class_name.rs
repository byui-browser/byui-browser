//! `Element.getElementsByClassName()` on script element wrappers.
//!
//! The method returns the element's descendants (never the element itself)
//! in tree order whose `class` attribute contains every whitespace-separated
//! token. Matching is case-sensitive, as in a no-quirks document. An empty or
//! all-whitespace argument matches nothing. The argument is converted with
//! `ToString`. Calling the method without an argument throws `TypeError`.
//!
//! Each match is the shared element wrapper, so it is identical (`===`) to
//! the wrapper from `getElementById()`. The collection also exposes
//! `item(index)` and `namedItem(name)`.
//!
//! Limitation: the DOM returns a live `HTMLCollection`. This returns a
//! snapshot taken when the method is called, so later class or tree changes
//! are not reflected in it.

use std::sync::{Arc, Mutex};

use html::{HTMLDocument, Namespace, NodeID, NodeKind};
use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::get_elements_by_tag_name::{ItemNames, collection};
use super::{thrown_exception, wrap_element};

/// Installs `Element.getElementsByClassName()` on the wrapper `object` for
/// `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.set_method(
        "getElementsByClassName",
        Arc::new(move |arguments| {
            let Some(argument) = arguments.first() else {
                return Err(thrown_exception(
                    "TypeError",
                    "Element.getElementsByClassName() requires 1 argument, but 0 were present",
                ));
            };
            let class_names = argument.to_string();
            let matches = {
                let locked = document.lock().unwrap();
                locked
                    .html_document()
                    .map(|source| matching_descendants(source, node, &class_names))
                    .unwrap_or_default()
            };
            log::debug!(
                "Element.getElementsByClassName({class_names:?}) matched {} elements",
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

/// Returns descendants of `root`, excluding `root`, whose class list contains
/// every token in `class_names`.
fn matching_descendants(
    source: &HTMLDocument,
    root: NodeID,
    class_names: &str,
) -> Vec<(NodeID, ItemNames)> {
    let wanted: Vec<&str> = class_names.split_ascii_whitespace().collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    let mut stack: Vec<NodeID> = source
        .node(root)
        .map(|node| node.get_children().iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(id) = stack.pop() {
        let Some(node) = source.node(id) else {
            continue;
        };
        if let NodeKind::Element(data) = node.get_kind()
            && let Some(class) = data
                .get_attributes()
                .iter()
                .find(|attribute| attribute.get_name() == "class")
        {
            let classes: Vec<&str> = class.get_value().split_ascii_whitespace().collect();
            if wanted.iter().all(|token| classes.contains(token)) {
                let attribute = |name: &str| {
                    data.get_attributes()
                        .iter()
                        .find(|attribute| attribute.get_name() == name)
                        .map(|attribute| attribute.get_value().clone())
                };
                let name = (*data.get_namespace() == Namespace::Html)
                    .then(|| attribute("name"))
                    .flatten();
                matches.push((id, (attribute("id"), name)));
            }
        }
        stack.extend(node.get_children().iter().rev().copied());
    }
    matches
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;
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

    const TREE: &str = "<div id='root' class='item'><span id='a' class='item'></span>\
                        <section id='s' class='item done'><i id='b' class='item'></i></section></div>\
                        <span id='outside' class='item'></span>";

    #[test]
    fn get_elements_by_class_name_returns_descendants_in_tree_order() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "var list = document.getElementById('root').getElementsByClassName('item'); \
                 list.length + ':' + list[0].getAttribute('id') + ',' + list[1].getAttribute('id') \
                 + ',' + list[2].getAttribute('id')"
            ),
            string("3:a,s,b")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var done = document.getElementById('root').getElementsByClassName('done item'); \
                 done.length + ':' + done[0].getAttribute('id') + ':' \
                 + document.getElementById('s').getElementsByClassName('item').length"
            ),
            string("1:s:1")
        );
    }

    #[test]
    fn get_elements_by_class_name_matches_case_sensitively_and_ignores_blank_input() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "var root = document.getElementById('root'); \
                 root.getElementsByClassName('ITEM').length + ':' \
                 + root.getElementsByClassName('').length + ':' \
                 + root.getElementsByClassName('  ').length"
            ),
            string("0:0:0")
        );
    }

    #[test]
    fn get_elements_by_class_name_returns_shared_wrappers_and_a_snapshot() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "var list = document.getElementById('root').getElementsByClassName('item'); \
                 var extra = document.createElement('span'); extra.className = 'item'; \
                 document.getElementById('root').append(extra); \
                 (list[0] === document.getElementById('a')) + ':' + list.length + ':' \
                 + document.getElementById('root').getElementsByClassName('item').length \
                 + ':' + (list.item(1) === document.getElementById('s')) \
                 + ':' + (list.namedItem('b') === document.getElementById('b')) \
                 + ':' + (list.item(9) === null)"
            ),
            string("true:3:4:true:true:true")
        );
    }

    #[test]
    fn get_elements_by_class_name_without_an_argument_throws_type_error() {
        let mut realm = realm_with(TREE);
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('root').getElementsByClassName(); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
    }
}
