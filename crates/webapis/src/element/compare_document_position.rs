//! `Node.compareDocumentPosition()` on script element wrappers.
//!
//! The bitmask follows the DOM Standard. `DOCUMENT_POSITION_CONTAINS` means
//! the argument contains this element, and `DOCUMENT_POSITION_CONTAINED_BY`
//! means this element contains the argument. Disconnected nodes also set
//! `DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC`; which of preceding or
//! following is set follows the nodes' arena ids, and that choice stays the
//! same for a pair of nodes.
//!
//! The constants are own read-only properties of each element wrapper because
//! there is no shared `Node` interface object. A missing argument or a
//! non-node throws `TypeError`. Attribute nodes are not represented, so the
//! attribute cases of the algorithm do not apply.

use std::sync::{Arc, Mutex};

use html::{HTMLDocument, NodeID};
use js::{Object, Value};

use crate::document::Document;
use crate::element::Element;

use super::{node_argument, thrown_exception};

/// `other` and `this` are in different trees.
const DISCONNECTED: u16 = 0x01;
/// `other` precedes `this` in tree order.
const PRECEDING: u16 = 0x02;
/// `other` follows `this` in tree order.
const FOLLOWING: u16 = 0x04;
/// `other` contains `this`.
const CONTAINS: u16 = 0x08;
/// `this` contains `other`.
const CONTAINED_BY: u16 = 0x10;
/// The preceding/following bit for disconnected nodes is engine-defined.
const IMPLEMENTATION_SPECIFIC: u16 = 0x20;

/// Node position constants, in the DOM Standard's declaration order.
const POSITION_CONSTANTS: [(&str, u16); 6] = [
    ("DOCUMENT_POSITION_DISCONNECTED", DISCONNECTED),
    ("DOCUMENT_POSITION_PRECEDING", PRECEDING),
    ("DOCUMENT_POSITION_FOLLOWING", FOLLOWING),
    ("DOCUMENT_POSITION_CONTAINS", CONTAINS),
    ("DOCUMENT_POSITION_CONTAINED_BY", CONTAINED_BY),
    (
        "DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC",
        IMPLEMENTATION_SPECIFIC,
    ),
];

/// Installs `Node.compareDocumentPosition()` and the position constants on
/// the wrapper `object` for `element`.
pub(super) fn install(object: &Object, element: &Element, document: &Arc<Mutex<Document>>) {
    for (name, value) in POSITION_CONSTANTS {
        object.set_read_only(name, Value::Number(f64::from(value)));
    }
    let node = element.node;
    let document = Arc::clone(document);
    object.set_method(
        "compareDocumentPosition",
        Arc::new(move |arguments| {
            let Some(value) = arguments.first() else {
                return Err(thrown_exception(
                    "TypeError",
                    "Element.compareDocumentPosition() requires 1 argument, but 0 were present",
                ));
            };
            let Some((other, _)) = node_argument(value) else {
                return Err(thrown_exception(
                    "TypeError",
                    "Element.compareDocumentPosition() expects a Node",
                ));
            };
            let position = {
                let locked = document.lock().unwrap();
                compare(&locked, node, other)
            };
            log::debug!("Element.compareDocumentPosition() = {position:#x}");
            Ok(Value::Number(f64::from(position)))
        }),
    );
}

/// Returns the DOM position bitmask of `other` relative to `this`.
fn compare(document: &Document, this: NodeID, other: NodeID) -> u16 {
    if this == other {
        return 0;
    }
    let Some(source) = document.html_document() else {
        return disconnected(this, other);
    };
    if source.get_node(this).is_none() || source.get_node(other).is_none() {
        return disconnected(this, other);
    }
    if root_of(source, this) != root_of(source, other) {
        return disconnected(this, other);
    }
    if is_ancestor(source, other, this) {
        return CONTAINS | PRECEDING;
    }
    if is_ancestor(source, this, other) {
        return CONTAINED_BY | FOLLOWING;
    }
    let order = tree_order(source, root_of(source, this));
    let other_at = order.iter().position(|id| *id == other);
    let this_at = order.iter().position(|id| *id == this);
    if other_at < this_at {
        PRECEDING
    } else {
        FOLLOWING
    }
}

/// Position of two nodes that do not share a root.
///
/// The preceding/following bit is chosen from the arena ids so the result is
/// stable and symmetric: if `a` reports `b` as preceding, `b` reports `a` as
/// following.
fn disconnected(this: NodeID, other: NodeID) -> u16 {
    let order = if other.get() < this.get() {
        PRECEDING
    } else {
        FOLLOWING
    };
    DISCONNECTED | IMPLEMENTATION_SPECIFIC | order
}

/// Walks parents until the node that has none.
fn root_of(source: &HTMLDocument, node: NodeID) -> NodeID {
    let mut current = node;
    while let Some(parent) = source.get_node(current).and_then(|data| data.get_parent()) {
        current = parent;
    }
    current
}

/// Reports whether `ancestor` is a strict ancestor of `node`.
fn is_ancestor(source: &HTMLDocument, ancestor: NodeID, node: NodeID) -> bool {
    let mut current = node;
    while let Some(parent) = source.get_node(current).and_then(|data| data.get_parent()) {
        if parent == ancestor {
            return true;
        }
        current = parent;
    }
    false
}

/// Lists `root` and its descendants in tree order.
fn tree_order(source: &HTMLDocument, root: NodeID) -> Vec<NodeID> {
    let mut order = Vec::new();
    fn visit(source: &HTMLDocument, node: NodeID, order: &mut Vec<NodeID>) {
        order.push(node);
        if let Some(data) = source.get_node(node) {
            for child in data.get_children() {
                visit(source, *child, order);
            }
        }
    }
    visit(source, root, &mut order);
    order
}

#[cfg(test)]
mod tests {
    use html::parse_raw_html;
    use js::{Realm, Value};

    use super::{CONTAINED_BY, CONTAINS, DISCONNECTED, FOLLOWING, PRECEDING};
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

    fn number(value: u16) -> Value {
        Value::Number(f64::from(value))
    }

    #[test]
    fn compare_document_position_reports_ancestors_descendants_and_siblings() {
        let mut realm = realm_with("<div id='p'><a id='a'></a><b id='b'></b></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 var a = document.getElementById('a'); \
                 var b = document.getElementById('b'); \
                 parent.compareDocumentPosition(parent) + ':' \
                 + parent.compareDocumentPosition(a) + ':' \
                 + a.compareDocumentPosition(parent) + ':' \
                 + a.compareDocumentPosition(b) + ':' \
                 + b.compareDocumentPosition(a) + ':' \
                 + a.compareDocumentPosition(document)"
            ),
            string(&format!(
                "0:{}:{}:{}:{}:{}",
                CONTAINED_BY | FOLLOWING,
                CONTAINS | PRECEDING,
                FOLLOWING,
                PRECEDING,
                CONTAINS | PRECEDING
            ))
        );
        assert_eq!(
            eval(&mut realm, "parent.DOCUMENT_POSITION_CONTAINED_BY"),
            number(CONTAINED_BY)
        );
        assert_eq!(
            eval(&mut realm, "parent.DOCUMENT_POSITION_DISCONNECTED"),
            number(DISCONNECTED)
        );
    }

    #[test]
    fn compare_document_position_marks_disconnected_nodes() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var parent = document.getElementById('p'); \
                 var created = document.createElement('i'); \
                 (parent.compareDocumentPosition(created) & parent.DOCUMENT_POSITION_DISCONNECTED) !== 0 \
                 && (parent.compareDocumentPosition(created) & parent.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC) !== 0 \
                 && (created.compareDocumentPosition(parent) & created.DOCUMENT_POSITION_DISCONNECTED) !== 0"
            ),
            Value::Boolean(true)
        );
    }

    #[test]
    fn compare_document_position_requires_a_node() {
        let mut realm = realm_with("<div id='p'></div>");
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('p').compareDocumentPosition(); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
        assert_eq!(
            eval(
                &mut realm,
                "var name = 'none'; \
                 try { document.getElementById('p').compareDocumentPosition(null); } \
                 catch (error) { name = error.name; } \
                 name"
            ),
            string("TypeError")
        );
    }
}
