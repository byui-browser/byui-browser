//! `Node.parentElement` on script element wrappers.
//!
//! The getter is live. It returns the shared wrapper of the parent when that
//! parent is an element, and `null` when the node is disconnected or its
//! parent is the document. Assignment is ignored.

use std::sync::{Arc, Mutex};

use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::node_view::{is_element, parent_id};
use super::wrap_element;

/// Installs `Node.parentElement` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.define_accessor(
        "parentElement",
        Arc::new(move || {
            let parent = {
                let locked = document.lock().unwrap();
                parent_id(&locked, node).filter(|parent| is_element(&locked, *parent))
            };
            match parent {
                Some(parent) => wrap_element(&document, parent),
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
    fn parent_element_is_the_element_parent_or_null() {
        let mut realm = realm_with("<div id='p'><span id='c'></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('c').parentElement === document.getElementById('p')"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('p').parentElement"),
            Value::Null
        );
        assert_eq!(
            eval(&mut realm, "document.createElement('b').parentElement"),
            Value::Null
        );
        eval(
            &mut realm,
            "var child = document.getElementById('c'); child.parentElement = null;",
        );
        assert_eq!(
            eval(
                &mut realm,
                "child.parentElement === document.getElementById('p')"
            ),
            Value::Boolean(true)
        );
    }
}
