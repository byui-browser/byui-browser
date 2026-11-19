//! `ParentNode.children` for elements on script element wrappers.
//!
//! The collection follows the DOM Standard's `HTMLCollection` interface: it
//! holds only the element children of the parent, in tree order, and is
//! **live**. Every read of `length`, an index, `item()`, or `namedItem()`
//! walks the current document tree, so later `appendChild`/`removeChild`
//! calls are visible through a collection obtained earlier. As required by
//! `[SameObject]`, `el.children` returns the same collection object each
//! time it is read on the same wrapper.
//!
//! Limitations:
//! - Supported named properties (`collection.someId`) are not exposed; use
//!   `namedItem()` instead. This avoids named properties shadowing the
//!   collection's own methods, which this engine's host property hooks would
//!   otherwise allow.
//! - The collection is a plain host object, not an `Array` and not an
//!   instance of an `HTMLCollection` interface object, so array methods and
//!   `instanceof HTMLCollection` are unavailable.

use std::sync::{Arc, Mutex};

use html::{Namespace, NodeID, NodeKind};
use js::{JsResult, Object, Value};

use crate::document::Document;
use crate::element::{Element, wrap_element};

/// Installs `ParentNode.children` for elements on the wrapper `object` for `element`.
///
/// The property is a read-only live collection of `element`'s element
/// children; see the module documentation for the supported surface.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    let collection = children_collection(element.node, document);
    object.set_read_only("children", Value::Object(collection));
}

/// Builds the live `HTMLCollection`-like object for the element children of `parent`.
fn children_collection(parent: NodeID, document: &Arc<Mutex<Document>>) -> Object {
    let collection = Object::new();

    let length_document = document.clone();
    collection.define_accessor(
        "length",
        Arc::new(move || {
            Ok(Value::Number(
                element_children(&length_document, parent).len() as f64,
            ))
        }),
        None,
    );

    let item_document = document.clone();
    collection.set_method(
        "item",
        Arc::new(move |arguments: &[Value]| {
            let index = unsigned_long(arguments.first().unwrap_or(&Value::Undefined));
            child_at(&item_document, parent, index)
        }),
    );

    let named_document = document.clone();
    collection.set_method(
        "namedItem",
        Arc::new(move |arguments: &[Value]| {
            let name = arguments
                .first()
                .map_or_else(|| "undefined".to_owned(), ToString::to_string);
            match named_child(&named_document, parent, &name) {
                Some(node) => wrap_element(&named_document, node),
                None => Ok(Value::Null),
            }
        }),
    );

    let get_document = document.clone();
    let keys_document = document.clone();
    collection.set_property_hooks(
        Arc::new(move |property| match array_index(property) {
            Some(index) => {
                let Some(&node) = element_children(&get_document, parent).get(index) else {
                    return Ok(None);
                };
                wrap_element(&get_document, node).map(Some)
            }
            None => Ok(None),
        }),
        // Indexed properties are read-only; writes to them are ignored as in
        // non-strict JavaScript. Other keys become ordinary own properties.
        Arc::new(|property, _value| Ok(array_index(property).is_some())),
        Arc::new(move || {
            Ok((0..element_children(&keys_document, parent).len())
                .map(|index| index.to_string())
                .collect())
        }),
    );

    collection
}

/// Returns the element children of `parent` in tree order.
///
/// The document lock is released before returning so callers can wrap the
/// IDs with [`wrap_element`], which locks the document itself.
fn element_children(document: &Arc<Mutex<Document>>, parent: NodeID) -> Vec<NodeID> {
    let document = document.lock().unwrap();
    let Some(source) = document.html_document() else {
        return Vec::new();
    };
    let Some(node) = source.node(parent) else {
        return Vec::new();
    };
    node.get_children()
        .iter()
        .copied()
        .filter(|&child| {
            source
                .node(child)
                .is_some_and(|child| matches!(child.get_kind(), NodeKind::Element(_)))
        })
        .collect()
}

/// Returns the wrapper for the element child at `index`, or `null` when out of range.
fn child_at(document: &Arc<Mutex<Document>>, parent: NodeID, index: usize) -> JsResult<Value> {
    match element_children(document, parent).get(index) {
        Some(&node) => wrap_element(document, node),
        None => Ok(Value::Null),
    }
}

/// Finds the first element child whose `id` is `name`, or, for HTML
/// elements, whose `name` attribute is `name` (DOM `namedItem()` steps).
fn named_child(document: &Arc<Mutex<Document>>, parent: NodeID, name: &str) -> Option<NodeID> {
    if name.is_empty() {
        return None;
    }
    let children = element_children(document, parent);
    let document = document.lock().unwrap();
    let source = document.html_document()?;
    children.into_iter().find(|&child| {
        let Some(NodeKind::Element(data)) = source.node(child).map(|node| node.get_kind()) else {
            return false;
        };
        data.get_attributes().iter().any(|attribute| {
            attribute.get_value() == name
                && (attribute.get_name() == "id"
                    || (attribute.get_name() == "name" && *data.get_namespace() == Namespace::Html))
        })
    })
}

/// Parses a canonical array index property name such as `"0"` or `"12"`.
fn array_index(property: &str) -> Option<usize> {
    if property.is_empty() || (property.len() > 1 && property.starts_with('0')) {
        return None;
    }
    if !property.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    property.parse().ok()
}

/// Converts a JavaScript value with WebIDL `unsigned long` semantics.
fn unsigned_long(value: &Value) -> usize {
    let number = match value {
        Value::Number(number) => *number,
        Value::Boolean(flag) => f64::from(u8::from(*flag)),
        Value::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                0.0
            } else {
                text.parse().unwrap_or(f64::NAN)
            }
        }
        Value::Null => 0.0,
        _ => f64::NAN,
    };
    if !number.is_finite() {
        return 0;
    }
    number.trunc().rem_euclid(4_294_967_296.0) as usize
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
    fn children_lists_only_element_children_in_order() {
        let mut realm =
            realm_with("<div id='p'>text<a id='one'></a><!-- c -->more<b id='two'></b> </div>");
        assert_eq!(
            eval(&mut realm, "document.getElementById('p').children.length"),
            Value::Number(2.0)
        );
        assert_eq!(
            eval(
                &mut realm,
                "var c = document.getElementById('p').children; c[0].getAttribute('id') + ',' + c[1].getAttribute('id')"
            ),
            string("one,two")
        );
        assert_eq!(eval(&mut realm, "c[2]"), Value::Undefined);
    }

    #[test]
    fn children_does_not_include_grandchildren() {
        let mut realm = realm_with("<div id='p'><span id='c'><i id='g'></i></span></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var c = document.getElementById('p').children; c.length + ':' + c[0].getAttribute('id')"
            ),
            string("1:c")
        );
    }

    #[test]
    fn children_entries_are_identical_to_get_element_by_id_wrappers() {
        let mut realm = realm_with("<div id='p'><a id='one'></a><b id='two'></b></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var c = document.getElementById('p').children; \
                 c[0] === document.getElementById('one') && \
                 c.item(1) === document.getElementById('two') && \
                 c.namedItem('two') === document.getElementById('two')"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn children_item_returns_null_when_out_of_range() {
        let mut realm = realm_with("<div id='p'><a id='one'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var c = document.getElementById('p').children; \
                 c.item(0).getAttribute('id') + ',' + c.item(1) + ',' + c.item(-1) + ',' + c.item('0').getAttribute('id')"
            ),
            string("one,null,null,one")
        );
    }

    #[test]
    fn children_named_item_matches_id_then_html_name_attribute() {
        let mut realm = realm_with(
            "<form id='p'><input name='user' id='first'><input name='pass'><span id=''></span></form>",
        );
        assert_eq!(
            eval(
                &mut realm,
                "var c = document.getElementById('p').children; \
                 c.namedItem('first').getAttribute('name') + ',' + \
                 c.namedItem('pass').getAttribute('name') + ',' + \
                 c.namedItem('user').getAttribute('id') + ',' + c.namedItem('missing') + ',' + c.namedItem('')"
            ),
            string("user,pass,first,null,null")
        );
    }

    #[test]
    fn children_is_the_same_live_object_across_mutations() {
        let mut realm = realm_with("<div id='p'><a id='one'></a></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var p = document.getElementById('p'); var c = p.children; c === p.children"
            ),
            Value::Boolean(true)
        );
        assert_eq!(
            eval(
                &mut realm,
                "var n = document.createElement('span'); n.setAttribute('id', 'two'); \
                 p.appendChild(n); c.length + ':' + p.children.length + ':' + c[1].getAttribute('id')"
            ),
            string("2:2:two")
        );
        assert_eq!(
            eval(
                &mut realm,
                "p.removeChild(document.getElementById('one')); \
                 p.children.length + ':' + c.length + ':' + c[0].getAttribute('id') + ':' + c.item(1)"
            ),
            string("1:1:two:null")
        );
    }

    #[test]
    fn children_of_an_element_without_element_children_is_empty() {
        let mut realm = realm_with("<p id='p'>only text</p>");
        assert_eq!(
            eval(
                &mut realm,
                "var c = document.getElementById('p').children; c.length + ':' + c.item(0)"
            ),
            string("0:null")
        );
    }

    #[test]
    fn array_index_accepts_only_canonical_indices() {
        assert_eq!(super::array_index("0"), Some(0));
        assert_eq!(super::array_index("12"), Some(12));
        assert_eq!(super::array_index("01"), None);
        assert_eq!(super::array_index("-1"), None);
        assert_eq!(super::array_index("length"), None);
        assert_eq!(super::array_index(""), None);
    }
}
