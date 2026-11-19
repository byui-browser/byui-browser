//! `Node.parentNode` on script element wrappers.
//!
//! The getter is live. An element parent is that element's shared wrapper.
//! A document parent is the registered `document` object, which is how a
//! node created by the HTML parser reports its parent. A disconnected node
//! returns `null`. Assignment is ignored.
//!
//! Limitation: when the document has not been registered with a realm, a
//! document parent reads as `null` because there is no script object for it.

use std::sync::{Arc, Mutex};

use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::node_view::{parent_id, wrap_any_node};

/// Installs `Node.parentNode` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.define_accessor(
        "parentNode",
        Arc::new(move || {
            let parent = parent_id(&document.lock().unwrap(), node);
            match parent {
                Some(parent) => wrap_any_node(&document, parent),
                None => Ok(Value::Null),
            }
        }),
        None,
    );
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

    #[test]
    fn parent_node_is_the_parent_element_or_the_document() {
        let mut realm = realm_with("<div id='p'><span id='c'></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('c').parentNode === document.getElementById('p') \
                 && document.getElementById('p').parentNode === document \
                 && document.createElement('b').parentNode === null"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn parent_node_follows_append_and_ignores_assignment() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var child = document.createElement('span'); \
                 document.getElementById('p').append(child); \
                 child.parentNode = null; \
                 child.parentNode === document.getElementById('p')"
            ),
            Value::Boolean(true)
        );
    }
}
