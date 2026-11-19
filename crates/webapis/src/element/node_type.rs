//! `Node.nodeType` for elements on script element wrappers.
//!
//! Every element wrapper exposes `nodeType` as the read-only number `1`
//! (`ELEMENT_NODE`) together with the DOM Standard's node type constants
//! (`ELEMENT_NODE` = 1 through `NOTATION_NODE` = 12), so scripts can write
//! `el.nodeType === el.ELEMENT_NODE`.
//!
//! Limitation: only elements have script wrappers in this engine. Text,
//! comment, document, and other node kinds are not exposed to scripts, so
//! `nodeType` is always `1` on the wrappers that exist. The constants are
//! installed as own read-only properties of each wrapper because there is no
//! shared `Node` interface object or prototype yet; `Node.ELEMENT_NODE` on a
//! global `Node` constructor is not available. Assignments to `nodeType` or
//! to the constants are silently ignored, matching non-strict script
//! behavior for read-only attributes.

use std::sync::{Arc, Mutex};

use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

/// `nodeType` value of an element node, per the DOM Standard.
const ELEMENT_NODE: u16 = 1;

/// Node type constants from the DOM Standard's `Node` interface, in order.
const NODE_TYPE_CONSTANTS: [(&str, u16); 12] = [
    ("ELEMENT_NODE", 1),
    ("ATTRIBUTE_NODE", 2),
    ("TEXT_NODE", 3),
    ("CDATA_SECTION_NODE", 4),
    ("ENTITY_REFERENCE_NODE", 5),
    ("ENTITY_NODE", 6),
    ("PROCESSING_INSTRUCTION_NODE", 7),
    ("COMMENT_NODE", 8),
    ("DOCUMENT_NODE", 9),
    ("DOCUMENT_TYPE_NODE", 10),
    ("DOCUMENT_FRAGMENT_NODE", 11),
    ("NOTATION_NODE", 12),
];

/// Installs `Node.nodeType` and the `Node` type constants on the wrapper
/// `object` for `element`.
///
/// `nodeType` is the read-only number `1` (`ELEMENT_NODE`); the constants
/// are read-only numbers. Only elements are wrapped for scripts, so the
/// element and document are not consulted.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let _ = (element, document);
    object.set_read_only("nodeType", Value::Number(f64::from(ELEMENT_NODE)));
    for (name, value) in NODE_TYPE_CONSTANTS {
        object.set_read_only(name, Value::Number(f64::from(value)));
    }
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use super::NODE_TYPE_CONSTANTS;
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
    fn node_type_is_element_node_for_get_element_by_id() {
        let mut realm = realm_with("<div id='a'><p id='b'></p></div>");
        assert_eq!(
            eval(&mut realm, "document.getElementById('a').nodeType"),
            Value::Number(1.0)
        );
        assert_eq!(
            eval(&mut realm, "document.getElementById('b').nodeType"),
            Value::Number(1.0)
        );
    }

    #[test]
    fn node_type_is_element_node_for_create_element() {
        let mut realm = realm_with("<div></div>");
        assert_eq!(
            eval(&mut realm, "document.createElement('span').nodeType"),
            Value::Number(1.0)
        );
    }

    #[test]
    fn node_type_compares_equal_to_element_node_constant() {
        let mut realm = realm_with("<div id='a'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "const el = document.getElementById('a'); el.nodeType === el.ELEMENT_NODE"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &mut realm,
                "const c = document.createElement('p'); c.nodeType === c.ELEMENT_NODE && c.nodeType !== c.TEXT_NODE"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn node_type_constants_match_dom_standard() {
        let mut realm = realm_with("<div id='a'></div>");
        for (name, value) in NODE_TYPE_CONSTANTS {
            assert_eq!(
                eval(&mut realm, &format!("document.getElementById('a').{name}")),
                Value::Number(f64::from(value)),
                "{name}"
            );
        }
        assert_eq!(
            eval(&mut realm, "document.createElement('b').NOTATION_NODE"),
            Value::Number(12.0)
        );
    }

    #[test]
    fn node_type_and_constants_ignore_assignment() {
        let mut realm = realm_with("<div id='a'></div>");
        eval(
            &mut realm,
            "const el = document.getElementById('a'); \
             el.nodeType = 3; el.ELEMENT_NODE = 99; el.TEXT_NODE = 0;",
        );
        assert_eq!(eval(&mut realm, "el.nodeType"), Value::Number(1.0));
        assert_eq!(eval(&mut realm, "el.ELEMENT_NODE"), Value::Number(1.0));
        assert_eq!(eval(&mut realm, "el.TEXT_NODE"), Value::Number(3.0));
        assert_eq!(
            eval(&mut realm, "delete el.nodeType"),
            Value::Boolean(false)
        );
        assert_eq!(eval(&mut realm, "el.nodeType"), Value::Number(1.0));
    }
}
