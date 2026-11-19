//! Shared node lookup and script wrappers for Element tree accessors.
//!
//! Element wrappers are reused from the document cache. Text and comment
//! nodes get a small character-data object (`nodeType`, `nodeName`, and
//! read-only `data` / `nodeValue` / `textContent`) so `firstChild` can return
//! them. Doctype nodes are not wrapped and read as `null`.

use std::sync::{Arc, Mutex};

use html::{NodeID, NodeKind};
use js::{JsResult, Object, Value};

use crate::document::Document;

use super::wrap_element;

/// Kind of node [`wrap_any_node`] knows how to expose to scripts.
enum WrappedKind {
    Element,
    Text,
    Comment,
    Document,
    Unsupported,
}

/// Returns the parent of `node`, if it has one.
pub(super) fn parent_id(document: &Document, node: NodeID) -> Option<NodeID> {
    document
        .html_document()
        .and_then(|source| source.node(node))
        .and_then(|node| node.get_parent())
}

/// Returns the first child of `node`, if it has one.
pub(super) fn first_child_id(document: &Document, node: NodeID) -> Option<NodeID> {
    document
        .html_document()
        .and_then(|source| source.node(node))
        .and_then(|node| node.get_children().first().copied())
}

/// Reports whether `node` is an element in `document`.
pub(super) fn is_element(document: &Document, node: NodeID) -> bool {
    matches!(
        document
            .html_document()
            .and_then(|source| source.node(node))
            .map(|node| node.get_kind()),
        Some(NodeKind::Element(_))
    )
}

/// Returns the script value for `node`.
///
/// Elements use their shared wrapper. The document node uses the registered
/// `document` object. Text and comment nodes use a cached character-data
/// object. Anything else, including a missing id, is `null`.
pub(super) fn wrap_any_node(document: &Arc<Mutex<Document>>, node: NodeID) -> JsResult<Value> {
    let kind = {
        let locked = document.lock().unwrap();
        let Some(source) = locked.html_document() else {
            return Ok(Value::Null);
        };
        let Some(data) = source.node(node) else {
            return Ok(Value::Null);
        };
        match data.get_kind() {
            NodeKind::Element(_) => WrappedKind::Element,
            NodeKind::Text(_) => WrappedKind::Text,
            NodeKind::Comment(_) => WrappedKind::Comment,
            NodeKind::Document => WrappedKind::Document,
            NodeKind::Doctype { .. } | NodeKind::None => WrappedKind::Unsupported,
        }
    };
    match kind {
        WrappedKind::Element => wrap_element(document, node),
        WrappedKind::Document => {
            let locked = document.lock().unwrap();
            Ok(locked
                .script_bindings()
                .map(|bindings| Value::Object(bindings.document))
                .unwrap_or(Value::Null))
        }
        WrappedKind::Text => Ok(Value::Object(character_data(document, node, 3, "#text"))),
        WrappedKind::Comment => Ok(Value::Object(character_data(document, node, 8, "#comment"))),
        WrappedKind::Unsupported => Ok(Value::Null),
    }
}

/// Returns the cached character-data wrapper for `node`, creating it on the
/// first request.
///
/// `data`, `nodeValue`, and `textContent` read the current node data and
/// ignore assignment. Writing those properties is not supported.
fn character_data(
    document: &Arc<Mutex<Document>>,
    node: NodeID,
    node_type: u16,
    node_name: &str,
) -> Object {
    if let Some(existing) = document.lock().unwrap().cached_script_element(node) {
        return existing;
    }
    let object = Object::new();
    object.set("__nodeId", Value::Number(node.get() as f64));
    object.set_read_only("nodeType", Value::Number(f64::from(node_type)));
    object.set_read_only("nodeName", Value::String(node_name.to_owned()));
    for property in ["data", "nodeValue", "textContent"] {
        let document = Arc::clone(document);
        object.define_accessor(
            property,
            Arc::new(move || {
                let text = document
                    .lock()
                    .unwrap()
                    .html_document()
                    .and_then(|source| source.text_content(node))
                    .unwrap_or_default();
                Ok(Value::String(text))
            }),
            None,
        );
    }
    document.lock().unwrap().remember_script_node(node, object)
}
