//! `Node.contains()` on script element wrappers.
//!
//! `contains(other)` is true when `other` is this element or a descendant of
//! it. `null` and a missing argument are false. A value that is not a node
//! throws `TypeError`.

use std::sync::{Arc, Mutex};

use html::NodeID;
use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::{node_argument, thrown_exception};

/// Installs `Node.contains(other)` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.set_method(
        "contains",
        Arc::new(move |arguments| {
            let other = match arguments.first() {
                None | Some(Value::Null) | Some(Value::Undefined) => {
                    return Ok(Value::Boolean(false));
                }
                Some(value) => match node_argument(value) {
                    Some((id, _)) => id,
                    None => {
                        return Err(thrown_exception(
                            "TypeError",
                            "Element.contains() expects a Node or null",
                        ));
                    }
                },
            };
            let contains = {
                let locked = document.lock().unwrap();
                node_contains(&locked, node, other)
            };
            log::debug!("Element.contains() is {contains}");
            Ok(Value::Boolean(contains))
        }),
    );
}

/// Reports whether `other` is an inclusive descendant of `this`.
fn node_contains(document: &Document, this: NodeID, other: NodeID) -> bool {
    if this == other {
        return true;
    }
    let Some(source) = document.html_document() else {
        return false;
    };
    let mut current = other;
    while let Some(parent) = source.get_node(current).and_then(|node| node.get_parent()) {
        if parent == this {
            return true;
        }
        current = parent;
    }
    false
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

    #[test]
    fn contains_is_true_for_self_and_descendants() {
        let mut realm = realm_with("<div id='p'>text<span id='c'><i id='g'></i></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 var child = document.getElementById('c'); \
                 var grand = document.getElementById('g'); \
                 parent.contains(parent) && parent.contains(child) && parent.contains(grand) \
                 && parent.contains(parent.firstChild) && child.contains(grand) \
                 && !child.contains(parent) && !parent.contains(null) && !parent.contains()"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn contains_rejects_a_non_node() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('p').contains('x'); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
    }
}
