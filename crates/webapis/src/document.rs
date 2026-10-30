//! Script-visible documents and the JavaScript `document` binding.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use html::{DOMTokenList, HTMLDocument, NodeID, NodeKind};
use js::{JsError, JsResult, Object, Realm, Value};

use crate::dom_exception::DomException;
use crate::element::{Element, element_object};
use crate::event::{
    EventTarget, EventTargetRef, abort_controller_constructor, add_event_listener,
    dispatch_event_with_target, event_constructor, remove_event_listener,
};
use crate::selector::{SelectorError, SimpleSelector, parse_selector_list};

/// Implements JavaScript `document.getElementById(id)`.
pub fn get_element_by_id(document: &Document, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(id)] = arguments else {
        return Err(JsError::new(
            "document.getElementById() expects one id string",
        ));
    };

    Ok(match document.get_element_by_id(id) {
        Some(element) => Value::Object(element_object(
            element.clone(),
            Arc::new(Mutex::new(document.clone())),
        )),
        None => Value::Null,
    })
}

/// Registers the DOM methods and `Event` constructor supported by this crate
/// on the global JavaScript environment.
pub fn register_document(realm: &mut Realm, document: Document) -> JsResult<()> {
    let document_object = Object::new();
    let shared_document = Arc::new(Mutex::new(document));
    let get_element_document = shared_document.clone();
    document_object.set_method(
        "getElementById",
        Arc::new(move |arguments| {
            let document = get_element_document.lock().unwrap();
            let [Value::String(id)] = arguments else {
                return Err(JsError::new(
                    "document.getElementById() expects one id string",
                ));
            };
            Ok(match document.get_element_by_id(id).cloned() {
                Some(element) => {
                    Value::Object(element_object(element, get_element_document.clone()))
                }
                None => Value::Null,
            })
        }),
    );
    let event_listener_document = shared_document.clone();
    document_object.set_method(
        "addEventListener",
        Arc::new(move |arguments| {
            let document = event_listener_document.lock().unwrap();
            add_event_listener(&document, arguments)
        }),
    );
    let remove_event_document = shared_document.clone();
    document_object.set_method(
        "removeEventListener",
        Arc::new(move |arguments| {
            let document = remove_event_document.lock().unwrap();
            remove_event_listener(&document, arguments)
        }),
    );
    let dispatch_event_document = shared_document;
    let dispatch_target = document_object.clone();
    document_object.set_method(
        "dispatchEvent",
        Arc::new(move |arguments| {
            let document = dispatch_event_document.lock().unwrap();
            dispatch_event_with_target(&document, &dispatch_target, arguments)
        }),
    );
    realm.register_global_function("Event", Arc::new(event_constructor))?;
    realm.register_global_function("AbortController", Arc::new(abort_controller_constructor))?;
    realm.register_global_value("document", Value::Object(document_object))
}

/// Script-visible `document` object.
#[derive(Debug, Clone)]
pub struct Document {
    pub(crate) elements: Vec<Element>,
    pub(crate) source_document: Option<HTMLDocument>,
    pub(crate) event_target: Arc<Mutex<EventTarget>>,
    pub(crate) event_targets: Arc<Mutex<HashMap<usize, EventTargetRef>>>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.elements == other.elements && self.source_document == other.source_document
    }
}

impl Eq for Document {}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            source_document: Some(HTMLDocument::new()),
            event_target: Arc::default(),
            event_targets: Arc::default(),
        }
    }

    /// Creates a script-visible document from the HTML document tree.
    pub fn from_html_document(document: &HTMLDocument) -> Self {
        Self {
            elements: collect_elements(document),
            source_document: Some(document.clone()),
            event_target: Arc::default(),
            event_targets: Arc::default(),
        }
    }

    /// Returns this document node's DOM `textContent` value.
    ///
    /// The DOM Standard defines `textContent` for a `Document` as `null`, so
    /// this method returns `None`. Use [`HTMLDocument::text_content`] with an
    /// element or character-data node ID to retrieve text from a node.
    pub fn text_content(&self) -> Option<String> {
        self.source_document
            .as_ref()
            .and_then(|document| document.text_content(document.root))
    }

    /// Implements `document.createElement(localName)` for an HTML document.
    ///
    /// The returned element is detached until a later tree mutation appends
    /// it. Names are validated according to the DOM Standard and normalized
    /// to ASCII lowercase. Invalid names return `InvalidCharacterError`.
    /// This Rust API owns the created node in the document's private arena.
    /// JavaScript method bindings and custom-element options are not supported.
    pub fn create_element(&mut self, local_name: &str) -> JsResult<Element> {
        let document = self.source_document.get_or_insert_with(HTMLDocument::new);

        let node = document
            .create_element(local_name)
            .map_err(|error| JsError::new(error.to_string()))?;

        let element = match document.node(node) {
            Some(html::Node {
                kind: NodeKind::Element(element),
                ..
            }) => element,
            _ => return Err(JsError::new("created node is not an element")),
        };

        Ok(Element {
            node,
            local_name: element.get_name().clone(),
            namespace: *element.get_namespace(),
            id_attr: None,
            attributes: element.get_attributes().clone(),
        })
    }

    /// `document.getElementById(id)`.
    ///
    pub fn get_element_by_id(&self, id: &str) -> Option<&Element> {
        self.elements
            .iter()
            .find(|element| element.id_attr.as_deref() == Some(id))
    }

    pub(crate) fn element_by_node(&self, node: NodeID) -> Option<&Element> {
        self.elements.iter().find(|element| element.node == node)
    }

    pub(crate) fn set_attribute(
        &mut self,
        node: NodeID,
        qualified_name: &str,
        value: &str,
    ) -> JsResult<Value> {
        let document = self.source_document.get_or_insert_with(HTMLDocument::new);
        document
            .set_attribute(node, qualified_name, value)
            .map_err(|error| JsError::new(error.to_string()))?;
        self.elements = collect_elements(document);
        Ok(Value::Undefined)
    }

    /// Returns this document's child nodes serialized as an HTML fragment.
    ///
    /// This getter uses the HTML fragment serialization behavior implemented
    /// by the backing arena. The HTML Standard exposes `innerHTML` on elements
    /// and shadow roots rather than on `Document`; this document-level helper
    /// serializes the document's children for the browser's current API model.
    /// It does not parse or replace markup, and template contents, shadow roots,
    /// and complete namespace-specific serialization are not supported.
    pub fn inner_html(&self) -> String {
        self.source_document
            .as_ref()
            .map_or_else(String::new, HTMLDocument::inner_html)
    }

    /// Implements the DOM `Element.classList` getter for an element in this
    /// document. The returned live token list mutates the element's `class`
    /// attribute through the DOM Standard operations.
    pub fn class_list(&mut self, element: NodeID) -> JsResult<DOMTokenList<'_>> {
        self.source_document
            .get_or_insert_with(HTMLDocument::new)
            .class_list(element)
            .map_err(|error| JsError::new(error.to_string()))
    }

    /// Removes an element node from this document's tree.
    ///
    /// The node and its descendants remain allocated and retain their stable
    /// IDs, but they are no longer reachable from the document root. Removing
    /// a detached node, the document root, or an invalid node ID is a no-op.
    pub fn remove(&mut self, node: NodeID) {
        let Some(document) = self.source_document.as_mut() else {
            return;
        };

        document.remove(node);
        self.elements = collect_elements(document);
    }

    /// Returns a static snapshot of matching element descendants in tree order.
    ///
    /// The document itself is not a candidate, detached nodes are excluded,
    /// and each matching node is returned once even if multiple selectors in
    /// a selector list match it. An invalid or unsupported selector returns a
    /// [`SelectorError`], corresponding to the DOM `SyntaxError` exception.
    /// Supported selectors are element names, `*`, IDs, classes, compound
    /// combinations of those forms, and comma-separated selector lists.
    pub fn query_selector_all(&self, selector: &str) -> Result<Vec<NodeID>, SelectorError> {
        let selectors = parse_selector_list(selector)?;
        let mut matches = Vec::new();
        let Some(document) = &self.source_document else {
            return Ok(matches);
        };

        fn collect(
            document: &HTMLDocument,
            parent: html::NodeID,
            selectors: &[SimpleSelector],
            matches: &mut Vec<NodeID>,
        ) {
            let Some(parent_node) = document.node(parent) else {
                return;
            };
            for child in &parent_node.children {
                let Some(node) = document.node(*child) else {
                    continue;
                };
                if let NodeKind::Element(element) = &node.kind {
                    if selectors.iter().any(|selector| selector.matches(element)) {
                        matches.push(*child);
                    }
                    collect(document, *child, selectors, matches);
                }
            }
        }

        collect(document, document.root, &selectors, &mut matches);
        Ok(matches)
    }

    /// Appends an existing node from this document to the document's children.
    ///
    /// Returns `child` after a successful append, matching the DOM
    /// `Node.appendChild()` return value. If `child` already has a parent, it
    /// is moved from that parent before being appended. The document hierarchy
    /// rules are checked before changing the tree; invalid insertions return a
    /// [`DomException`]. The current DOM model supports existing node IDs only;
    /// it does not yet provide `DocumentFragment` or JavaScript `Node` values.
    pub fn append_child(&mut self, child: NodeID) -> Result<NodeID, DomException> {
        if self.source_document.is_none() {
            self.source_document = Some(HTMLDocument::new());
        }
        let source = self.source_document.as_ref().unwrap();
        let root = source.root;
        let child_id = child;
        let child_node = source.node(child_id).ok_or(DomException::NotFoundError)?;

        if child_id == root || matches!(child_node.kind, NodeKind::Document) {
            return Err(DomException::HierarchyRequestError);
        }

        match &child_node.kind {
            NodeKind::Comment(_) | NodeKind::Element(_) | NodeKind::Doctype { .. } => {}
            NodeKind::Text(_) => return Err(DomException::HierarchyRequestError),
            NodeKind::Document => return Err(DomException::HierarchyRequestError),
            NodeKind::None => return Err(DomException::HierarchyRequestError),
        }

        let root_node = source
            .node(root)
            .ok_or(DomException::HierarchyRequestError)?;
        if !matches!(root_node.kind, NodeKind::Document) {
            return Err(DomException::HierarchyRequestError);
        }

        for sibling in root_node
            .children
            .iter()
            .copied()
            .filter(|sibling| *sibling != child_id)
        {
            let Some(sibling) = source.node(sibling) else {
                return Err(DomException::HierarchyRequestError);
            };
            match (&child_node.kind, &sibling.kind) {
                (NodeKind::Element(_), NodeKind::Element(_))
                | (NodeKind::Doctype { .. }, NodeKind::Doctype { .. })
                | (NodeKind::Doctype { .. }, NodeKind::Element(_)) => {
                    return Err(DomException::HierarchyRequestError);
                }
                _ => {}
            }
        }

        if let Some(parent) = child_node.parent {
            let Some(parent_node) = source.node(parent) else {
                return Err(DomException::NotFoundError);
            };
            if !parent_node.children.contains(&child_id) {
                return Err(DomException::NotFoundError);
            }
        }

        let document = self.source_document.as_mut().unwrap();
        if let Some(parent) = document.nodes[child_id.get()].parent {
            let siblings = &mut document.nodes[parent.get()].children;
            let Some(position) = siblings.iter().position(|sibling| *sibling == child_id) else {
                return Err(DomException::NotFoundError);
            };
            siblings.remove(position);
        }
        document.nodes[child_id.get()].parent = Some(root);
        document.nodes[root.get()].children.push(child_id);
        self.elements = collect_elements(document);

        Ok(child)
    }
}

fn collect_elements(document: &HTMLDocument) -> Vec<Element> {
    fn collect(document: &HTMLDocument, parent: NodeID, elements: &mut Vec<Element>) {
        let Some(node) = document.node(parent) else {
            return;
        };

        for child in &node.children {
            if let Some(node) = document.node(*child) {
                if let NodeKind::Element(element) = &node.kind {
                    let id_attr = element
                        .get_attributes()
                        .iter()
                        .find(|attribute| attribute.get_name() == "id")
                        .map(|attribute| attribute.get_value().clone());
                    elements.push(Element {
                        node: *child,
                        local_name: element.get_name().clone(),
                        namespace: *element.get_namespace(),
                        id_attr,
                        attributes: element.get_attributes().clone(),
                    });
                }
                collect(document, *child, elements);
            }
        }
    }

    let mut elements = Vec::new();
    collect(document, document.root, &mut elements);
    elements
}

#[cfg(test)]
mod tests {
    use html::{Attribute, ElementData, HTMLDocument, Namespace, NodeID, NodeKind, parse_raw_html};

    use super::*;
    use crate::dom_exception::DomException;

    #[test]
    fn empty_document_has_no_element_by_id() {
        assert_eq!(Document::new().get_element_by_id("main"), None);
    }

    #[test]
    fn document_text_content_is_null_per_dom_standard() {
        let empty = Document::new();
        assert_eq!(empty.text_content(), None);

        let parsed =
            Document::from_html_document(&parse_raw_html("<main>document text</main>".to_owned()));
        assert_eq!(parsed.text_content(), None);
    }

    #[test]
    fn document_get_element_by_id_uses_tree_order_and_exact_matching() {
        let html_document = parse_raw_html(
            "<section id='target'><span id='nested'></span></section><p id='nested'></p>"
                .to_owned(),
        );
        let document = Document::from_html_document(&html_document);

        let element = document
            .get_element_by_id("nested")
            .expect("nested element");
        assert_eq!(element.node.get(), 2);
        assert_eq!(document.get_element_by_id("NESTED"), None);
    }

    #[test]
    fn create_element_lowercases_and_returns_a_detached_html_element() {
        let mut document = Document::new();

        let element = document.create_element("CuStOm-Widget").unwrap();

        assert_eq!(element.local_name, "custom-widget");
        assert_eq!(element.node.get(), 1);
        assert_eq!(element.id_attr, None);
    }

    #[test]
    fn create_element_rejects_invalid_local_names() {
        let mut document = Document::new();

        for name in ["", "1div", "div name", "div/name", "div>"] {
            assert_eq!(
                document.create_element(name).unwrap_err().to_string(),
                "InvalidCharacterError",
                "expected {name:?} to be rejected"
            );
        }
    }

    #[test]
    fn created_elements_preserve_parsed_nodes_and_stay_out_of_queries() {
        let mut document =
            Document::from_html_document(&parse_raw_html("<div id='existing'></div>".into()));
        let first = document.create_element("DIV").unwrap();
        let second = document.create_element("span").unwrap();
        assert_eq!(first.node, NodeID::new(2));
        assert_eq!(second.node, NodeID::new(3));
        assert_eq!(
            document.query_selector_all("*").unwrap(),
            vec![NodeID::new(1)]
        );
        assert_eq!(
            document.get_element_by_id("existing").unwrap().node,
            NodeID::new(1)
        );
        let source = document.source_document.as_ref().unwrap();
        assert_eq!(source.node(NodeID::new(2)).unwrap().parent, None);
        assert_eq!(source.node(NodeID::new(3)).unwrap().parent, None);
    }

    #[test]
    fn document_remove_detaches_the_node_from_queries() {
        let html_document = parse_raw_html(
            "<section><span id='target'></span></section><p id='other'></p>".to_owned(),
        );
        let mut document = Document::from_html_document(&html_document);
        let section = NodeID::new(1);

        document.remove(section);

        assert_eq!(document.get_element_by_id("target"), None);
        assert_eq!(
            document.get_element_by_id("other").unwrap().node,
            NodeID::new(3)
        );
        assert_eq!(document.query_selector_all("span").unwrap(), Vec::new());
    }

    #[test]
    fn document_remove_of_detached_or_root_nodes_is_a_no_op() {
        let html_document = parse_raw_html("<div id='target'></div>".to_owned());
        let mut document = Document::from_html_document(&html_document);

        document.remove(NodeID::new(99));
        document.remove(NodeID::new(0));

        assert!(document.get_element_by_id("target").is_some());
    }

    #[test]
    fn document_query_selector_all_returns_matching_descendants_in_tree_order() {
        let html_document = parse_raw_html(
            "<main><article class='post target'><span class='target'></span></article><p class='target'></p></main>"
                .to_owned(),
        );
        let document = Document::from_html_document(&html_document);

        assert_eq!(
            document.query_selector_all(".target, article").unwrap(),
            vec![NodeID::new(2), NodeID::new(3), NodeID::new(4)]
        );
        assert_eq!(
            document.query_selector_all("*").unwrap(),
            vec![
                NodeID::new(1),
                NodeID::new(2),
                NodeID::new(3),
                NodeID::new(4)
            ]
        );
    }

    #[test]
    fn document_query_selector_all_rejects_invalid_selectors() {
        let document = Document::from_html_document(&parse_raw_html("<div></div>".into()));

        assert!(document.query_selector_all("").is_err());
        assert!(document.query_selector_all("div,").is_err());
        assert_eq!(document.query_selector_all(".missing").unwrap(), Vec::new());
    }

    #[test]
    fn document_append_child_adds_a_detached_element_and_returns_its_id() {
        let mut html_document = HTMLDocument::new();
        let html_child = html_document.push_node(
            NodeKind::Element(ElementData::new(
                "main".into(),
                Namespace::Html,
                vec![Attribute::new("id".into(), "appended".into())],
            )),
            None,
        );
        let mut document = Document::from_html_document(&html_document);
        let child = html_child;

        assert_eq!(document.append_child(child), Ok(child));
        let source = document.source_document.as_ref().unwrap();
        assert_eq!(source.node(source.root).unwrap().children, vec![html_child]);
        assert_eq!(document.get_element_by_id("appended").unwrap().node, child);
    }

    #[test]
    fn document_append_child_moves_a_child_from_its_old_parent() {
        let mut html_document = HTMLDocument::new();
        let element = html_document.push_node(
            NodeKind::Element(ElementData::new("main".into(), Namespace::Html, Vec::new())),
            None,
        );
        let comment = html_document.push_node(NodeKind::Comment("move me".into()), None);
        html_document.append_child(html_document.root, element);
        html_document.append_child(element, comment);
        let mut document = Document::from_html_document(&html_document);
        let comment_id = comment;

        assert_eq!(document.append_child(comment_id), Ok(comment_id));
        let source = document.source_document.as_ref().unwrap();
        assert_eq!(source.node(element).unwrap().children, Vec::new());
        assert_eq!(
            source.node(source.root).unwrap().children,
            vec![element, comment]
        );
        assert_eq!(source.node(comment).unwrap().parent, Some(source.root));
    }

    #[test]
    fn document_append_child_rejects_invalid_hierarchy_without_mutating_the_tree() {
        let mut html_document = HTMLDocument::new();
        let existing_element = html_document.push_node(
            NodeKind::Element(ElementData::new("main".into(), Namespace::Html, Vec::new())),
            None,
        );
        let second_element = html_document.push_node(
            NodeKind::Element(ElementData::new(
                "aside".into(),
                Namespace::Html,
                Vec::new(),
            )),
            None,
        );
        html_document.append_child(html_document.root, existing_element);
        let mut document = Document::from_html_document(&html_document);
        let before = document.source_document.clone();

        assert_eq!(
            document.append_child(second_element),
            Err(DomException::HierarchyRequestError)
        );
        assert_eq!(document.source_document, before);
    }

    #[test]
    fn document_append_child_reports_unknown_node_ids() {
        let mut document = Document::new();

        assert_eq!(
            document.append_child(NodeID::new(99)),
            Err(DomException::NotFoundError)
        );
    }

    #[test]
    fn document_inner_html_serializes_its_children() {
        let html_document = parse_raw_html(
            "<!doctype html><main id=\"content\"><p>A &amp; B</p><!-- note --><br></main>"
                .to_owned(),
        );
        let document = Document::from_html_document(&html_document);

        assert_eq!(
            document.inner_html(),
            "<!DOCTYPE html><main id=\"content\"><p>A &amp; B</p><!-- note --><br></main>"
        );
    }
}
