//! `Node.firstChild` on script element wrappers.
//!
//! The getter is live: each read returns the element's current first child.
//! An element child is the shared element wrapper. A text or comment child is
//! a character-data object (see [`super::node_view`]). A missing child is
//! `null`. Assignment is ignored.
//!
//! Limitation: a doctype child is not wrapped and reads as `null`.

use std::sync::{Arc, Mutex};

use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::node_view::{first_child_id, wrap_any_node};

/// Installs `Node.firstChild` on the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let node = element.node;
    let document = Arc::clone(document);
    object.define_accessor(
        "firstChild",
        Arc::new(move || {
            let child = first_child_id(&document.lock().unwrap(), node);
            match child {
                Some(child) => wrap_any_node(&document, child),
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

    fn string(value: &str) -> Value {
        Value::String(value.to_owned())
    }

    #[test]
    fn first_child_returns_text_before_an_element_child() {
        let mut realm = realm_with("<div id='p'>hello<span id='s'></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var child = document.getElementById('p').firstChild; \
                 child.nodeType + ':' + child.nodeName + ':' + child.data + ':' + child.nodeValue \
                 + ':' + (child === document.getElementById('p').firstChild)"
            ),
            string("3:#text:hello:hello:true")
        );
    }

    #[test]
    fn first_child_of_an_element_is_the_shared_wrapper() {
        let mut realm = realm_with("<div id='p'><span id='s'></span>tail</div>");
        assert_eq!(
            eval(
                &mut realm,
                "document.getElementById('p').firstChild === document.getElementById('s')"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn first_child_returns_a_comment_and_null_when_empty() {
        let mut realm = realm_with("<div id='c'><!--note--><span></span></div><div id='e'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var comment = document.getElementById('c').firstChild; \
                 comment.nodeType + ':' + comment.data"
            ),
            string("8:note")
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('e').firstChild"),
            Value::Null
        );
        eval(
            &mut realm,
            "document.getElementById('e').firstChild = document.getElementById('c');",
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('e').firstChild"),
            Value::Null
        );
    }
}
