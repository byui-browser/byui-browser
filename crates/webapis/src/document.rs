//! Script-visible documents and the JavaScript `document` binding.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use html::{DOMTokenList, HTMLDocument, NodeID, NodeKind};
use js::{JsError, JsResult, Object, Realm, Value};

use crate::custom_elements::CustomElementRegistry;
use crate::dom_exception::DomException;
use crate::element::{
    Element, element_object, invoke_attribute_changed_callback, invoke_lifecycle_callback,
};
use crate::event::{
    EventTarget, EventTargetRef, abort_controller_constructor, add_event_listener,
    dispatch_event_with_target, event_constructor, remove_event_listener,
};
use crate::selector::{SelectorError, SimpleSelector, parse_selector_list};

/// Implements JavaScript `document.getElementById(id)`.
pub fn get_element_by_id(document: &Document, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(id)] = arguments else {
        log::debug!("document.getElementById() called with invalid arguments");
        return Err(JsError::new(
            "document.getElementById() expects one id string",
        ));
    };

    log::debug!("document.getElementById({id:?})");
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
///
/// The same `Document` object is exposed as both the global `document` and
/// `window.document`, matching the HTML Standard's readonly
/// `Window.document` attribute. Assignment to `window.document` cannot replace
/// the reference; strict-mode assignment errors are not modeled by the
/// JavaScript engine.
pub fn register_document(realm: &mut Realm, document: Document) -> JsResult<()> {
    log::debug!(
        "registering document bindings ({} elements)",
        document.elements.len()
    );
    let document_object = Object::new();
    let shared_document = Arc::new(Mutex::new(document));
    let custom_elements_registry =
        realm
            .registered_global_value("customElements")
            .and_then(|value| match value {
                Value::Object(registry) => Some(registry),
                _ => None,
            });
    let custom_elements_registry = match custom_elements_registry {
        Some(registry) => registry,
        None => {
            let registry = CustomElementRegistry::new().object();
            realm.register_global_value("customElements", Value::Object(registry.clone()))?;
            registry
        }
    };
    if let Some(Value::Function(define)) = custom_elements_registry.get("define") {
        let define_document = shared_document.clone();
        let define_registry = custom_elements_registry.clone();
        custom_elements_registry.set_method(
            "define",
            Arc::new(move |arguments| {
                let result = define.call(arguments)?;
                if let Some(Value::String(name)) = arguments.first() {
                    let candidates = define_document
                        .lock()
                        .unwrap()
                        .elements
                        .iter()
                        .filter(|element| element.local_name == *name)
                        .cloned()
                        .collect::<Vec<_>>();
                    for element in candidates {
                        let _ = script_element_object(&define_document, &define_registry, element);
                    }
                }
                Ok(result)
            }),
        );
    }
    let upgrade_document = shared_document.clone();
    let upgrade_registry = custom_elements_registry.clone();
    custom_elements_registry.set_method(
        "upgrade",
        Arc::new(move |arguments| {
            let [Value::Object(root)] = arguments else {
                return Err(JsError::new(
                    "TypeError: CustomElementRegistry.upgrade() expects a Node",
                ));
            };
            let Some(Value::Number(root_id)) = root.get("__nodeId") else {
                return Err(JsError::new(
                    "TypeError: CustomElementRegistry.upgrade() expects a Node",
                ));
            };
            let elements = upgrade_document
                .lock()
                .unwrap()
                .inclusive_descendant_elements(NodeID::new(root_id as usize));
            if elements.is_empty() {
                return Err(JsError::new(
                    "TypeError: CustomElementRegistry.upgrade() expects a Node",
                ));
            }
            for element in elements {
                let _ = script_element_object(&upgrade_document, &upgrade_registry, element);
            }
            Ok(Value::Undefined)
        }),
    );
    let get_element_document = shared_document.clone();
    let get_element_registry = custom_elements_registry.clone();
    document_object.set_method(
        "getElementById",
        Arc::new(move |arguments| {
            let [Value::String(id)] = arguments else {
                log::debug!("document.getElementById() called with invalid arguments");
                return Err(JsError::new(
                    "document.getElementById() expects one id string",
                ));
            };
            let element = {
                get_element_document
                    .lock()
                    .unwrap()
                    .get_element_by_id(id)
                    .cloned()
            };
            log::debug!("document.getElementById({id:?}) bridged to the DOM");
            Ok(match element {
                Some(element) => Value::Object(script_element_object(
                    &get_element_document,
                    &get_element_registry,
                    element,
                )?),
                None => Value::Null,
            })
        }),
    );
    let create_element_document = shared_document.clone();
    let create_element_registry = custom_elements_registry.clone();
    document_object.set_method(
        "createElement",
        Arc::new(move |arguments| {
            let [Value::String(local_name)] = arguments else {
                return Err(JsError::new(
                    "document.createElement() expects a local name",
                ));
            };
            let element = create_element_document
                .lock()
                .unwrap()
                .create_element(local_name)?;
            let object =
                script_element_object(&create_element_document, &create_element_registry, element)?;
            Ok(Value::Object(object))
        }),
    );
    let append_document = shared_document.clone();
    document_object.set_method(
        "appendChild",
        Arc::new(move |arguments| {
            let ([Value::Object(child)] | [Value::Array(child)]) = arguments else {
                return Err(JsError::new("document.appendChild() expects an element"));
            };
            let Some(Value::Number(child_id)) = child.get("__nodeId") else {
                return Err(JsError::new("document.appendChild() expects an element"));
            };
            let child_id = NodeID::new(child_id as usize);
            let was_connected = append_document.lock().unwrap().is_connected(child_id);
            if was_connected {
                invoke_lifecycle_callback(child, "disconnectedCallback");
            }
            append_document
                .lock()
                .unwrap()
                .append_child(child_id)
                .map_err(|error| JsError::new(error.to_string()))?;
            invoke_lifecycle_callback(child, "connectedCallback");
            Ok(Value::Object(child.clone()))
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
    let document_value = Value::Object(document_object);
    let window_object = match realm.registered_global_value("window") {
        Some(Value::Object(window)) => window,
        _ => Object::new(),
    };
    window_object.set_read_only("document", document_value.clone());
    realm.register_global_value("document", document_value)?;
    if realm.registered_global_value("window").is_none() {
        realm.register_global_value("window", Value::Object(window_object))?;
    }
    Ok(())
}

/// Registers a document on an already-created window.
///
/// This compatibility entry point preserves the browser crate's older
/// registration boundary. `window` must be the same object registered in
/// `realm`; the current document registration resolves that object from the
/// realm so the document cannot be attached to a different global.
pub fn register_document_on_window(
    realm: &mut Realm,
    document: Document,
    window: Object,
) -> JsResult<()> {
    let _ = window;
    register_document(realm, document)
}

/// Script-visible `document` object.
#[derive(Debug, Clone)]
pub struct Document {
    pub(crate) elements: Vec<Element>,
    pub(crate) source_document: Option<HTMLDocument>,
    pub(crate) event_target: Arc<Mutex<EventTarget>>,
    pub(crate) event_targets: Arc<Mutex<HashMap<usize, EventTargetRef>>>,
    custom_element_nodes: HashSet<NodeID>,
    script_elements: HashMap<NodeID, Object>,
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.elements == other.elements
            && self.source_document == other.source_document
            && self.custom_element_nodes == other.custom_element_nodes
            && self.script_elements == other.script_elements
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
            custom_element_nodes: HashSet::new(),
            script_elements: HashMap::new(),
        }
    }

    /// Creates a script-visible document from the HTML document tree.
    pub fn from_html_document(document: &HTMLDocument) -> Self {
        Self {
            elements: collect_elements(document),
            source_document: Some(document.clone()),
            event_target: Arc::default(),
            event_targets: Arc::default(),
            custom_element_nodes: HashSet::new(),
            script_elements: HashMap::new(),
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

        let node = document.create_element(local_name).map_err(|error| {
            log::debug!("createElement({local_name:?}) failed: {error}");
            JsError::new(error.to_string())
        })?;
        log::debug!("document.createElement({local_name:?})");

        let element = match document.node(node).map(|node| node.get_kind()) {
            Some(NodeKind::Element(element)) => element,
            _ => {
                log::error!("createElement({local_name:?}) produced a non-element node");
                return Err(JsError::new("created node is not an element"));
            }
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

    pub(crate) fn element_snapshot(&self, node: NodeID) -> Option<Element> {
        let node_data = self.source_document.as_ref()?.node(node)?;
        let NodeKind::Element(element) = node_data.get_kind() else {
            return None;
        };
        let id_attr = element
            .get_attributes()
            .iter()
            .find(|attribute| attribute.get_name() == "id")
            .map(|attribute| attribute.get_value().clone());
        Some(Element {
            node,
            local_name: element.get_name().clone(),
            namespace: *element.get_namespace(),
            id_attr,
            attributes: element.get_attributes().clone(),
        })
    }

    fn mark_custom_element_upgraded(&mut self, node: NodeID) -> bool {
        self.custom_element_nodes.insert(node)
    }

    fn inclusive_descendant_elements(&self, root: NodeID) -> Vec<Element> {
        fn collect(document: &Document, node: NodeID, elements: &mut Vec<Element>) {
            let Some(source) = &document.source_document else {
                return;
            };
            let Some(node_data) = source.node(node) else {
                return;
            };
            if let NodeKind::Element(element) = node_data.get_kind() {
                let id_attr = element
                    .get_attributes()
                    .iter()
                    .find(|attribute| attribute.get_name() == "id")
                    .map(|attribute| attribute.get_value().clone());
                elements.push(Element {
                    node,
                    local_name: element.get_name().clone(),
                    namespace: *element.get_namespace(),
                    id_attr,
                    attributes: element.get_attributes().clone(),
                });
            }
            for child in node_data.get_children() {
                collect(document, *child, elements);
            }
        }

        let mut elements = Vec::new();
        collect(self, root, &mut elements);
        elements
    }

    pub(crate) fn is_connected(&self, node: NodeID) -> bool {
        let Some(document) = &self.source_document else {
            return false;
        };
        let mut current = node;
        while let Some(parent) = document.node(current).and_then(|node| node.get_parent()) {
            if parent == document.root {
                return true;
            }
            current = parent;
        }
        false
    }

    pub(crate) fn set_attribute(
        &mut self,
        node: NodeID,
        qualified_name: &str,
        value: &str,
    ) -> JsResult<Value> {
        log::debug!(
            "setAttribute {qualified_name:?} ({} bytes) bridged to the DOM",
            value.len()
        );
        let document = self.source_document.get_or_insert_with(HTMLDocument::new);
        document
            .set_attribute(node, qualified_name, value)
            .map_err(|error| {
                log::debug!("setAttribute {qualified_name:?} failed: {error}");
                JsError::new(error.to_string())
            })?;
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

        log::debug!("removing node from the DOM");
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
        log::debug!("querySelectorAll({selector:?})");
        let selectors = parse_selector_list(selector).inspect_err(|error| {
            log::debug!("querySelectorAll({selector:?}) rejected: {error:?}");
        })?;
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
            for child in parent_node.get_children() {
                let Some(node) = document.node(*child) else {
                    continue;
                };
                if let NodeKind::Element(element) = node.get_kind() {
                    if selectors.iter().any(|selector| selector.matches(element)) {
                        matches.push(*child);
                    }
                    collect(document, *child, selectors, matches);
                }
            }
        }

        collect(document, document.root, &selectors, &mut matches);
        log::debug!(
            "querySelectorAll({selector:?}) matched {} nodes",
            matches.len()
        );
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
        log::debug!("document.appendChild bridged to the DOM");
        let child_node = source.node(child_id).ok_or_else(|| {
            log::debug!("appendChild failed: child node not found");
            DomException::NotFoundError
        })?;

        if child_id == root || matches!(child_node.get_kind(), NodeKind::Document) {
            log::debug!("appendChild failed: cannot append a document node");
            return Err(DomException::HierarchyRequestError);
        }

        match child_node.get_kind() {
            NodeKind::Comment(_) | NodeKind::Element(_) | NodeKind::Doctype { .. } => {}
            NodeKind::Text(_) | NodeKind::Document | NodeKind::None => {
                log::debug!("appendChild failed: node kind cannot be a document child");
                return Err(DomException::HierarchyRequestError);
            }
        }

        let root_node = source
            .node(root)
            .ok_or(DomException::HierarchyRequestError)?;
        if !matches!(root_node.get_kind(), NodeKind::Document) {
            return Err(DomException::HierarchyRequestError);
        }

        for sibling in root_node
            .get_children()
            .iter()
            .copied()
            .filter(|sibling| *sibling != child_id)
        {
            let Some(sibling) = source.node(sibling) else {
                return Err(DomException::HierarchyRequestError);
            };
            match (child_node.get_kind(), sibling.get_kind()) {
                (NodeKind::Element(_), NodeKind::Element(_))
                | (NodeKind::Doctype { .. }, NodeKind::Doctype { .. })
                | (NodeKind::Doctype { .. }, NodeKind::Element(_)) => {
                    log::debug!("appendChild failed: document would have duplicate children");
                    return Err(DomException::HierarchyRequestError);
                }
                _ => {}
            }
        }

        if let Some(parent) = child_node.get_parent() {
            let Some(parent_node) = source.node(parent) else {
                return Err(DomException::NotFoundError);
            };
            if !parent_node.get_children().contains(&child_id) {
                return Err(DomException::NotFoundError);
            }
        }

        let document = self.source_document.as_mut().unwrap();
        if let Some(parent) = document.nodes[child_id.get()].get_parent() {
            let siblings = document.nodes[parent.get()].get_children().to_vec();
            let Some(position) = siblings.iter().position(|sibling| *sibling == child_id) else {
                return Err(DomException::NotFoundError);
            };
            let mut updated_siblings = siblings;
            updated_siblings.remove(position);
            document.nodes[parent.get()].set_children(updated_siblings);
        }
        document.nodes[child_id.get()].set_parent(Some(root));
        let mut root_children = document.nodes[root.get()].get_children().clone();
        root_children.push(child_id);
        document.nodes[root.get()].set_children(root_children);
        self.elements = collect_elements(document);
        log::debug!(
            "appendChild done; document has {} elements",
            self.elements.len()
        );

        Ok(child)
    }

    /// Appends one element below another element in the document tree.
    pub(crate) fn append_child_to(&mut self, parent: NodeID, child: NodeID) -> JsResult<NodeID> {
        let document = self
            .source_document
            .as_mut()
            .ok_or_else(|| JsError::new("The document has no DOM tree"))?;
        if parent == child {
            return Err(JsError::new("HierarchyRequestError"));
        }
        if !matches!(
            document.node(parent).map(|node| node.get_kind()),
            Some(NodeKind::Element(_))
        ) || !matches!(
            document.node(child).map(|node| node.get_kind()),
            Some(NodeKind::Element(_))
        ) {
            return Err(JsError::new("HierarchyRequestError"));
        }
        let mut ancestor = Some(parent);
        while let Some(node) = ancestor {
            if node == child {
                return Err(JsError::new("HierarchyRequestError"));
            }
            ancestor = document.node(node).and_then(|node| node.get_parent());
        }
        if let Some(old_parent) = document.node(child).and_then(|node| node.get_parent()) {
            let mut siblings = document
                .node(old_parent)
                .ok_or_else(|| JsError::new("NotFoundError"))?
                .get_children()
                .clone();
            siblings.retain(|sibling| *sibling != child);
            document.nodes[old_parent.get()].set_children(siblings);
        }
        document.nodes[child.get()].set_parent(Some(parent));
        let mut children = document.nodes[parent.get()].get_children().clone();
        children.push(child);
        document.nodes[parent.get()].set_children(children);
        self.elements = collect_elements(document);
        Ok(child)
    }
}

fn upgrade_element(
    document: &mut Document,
    registry: &Object,
    element: &Element,
    object: &Object,
) -> JsResult<Option<Vec<(String, String)>>> {
    let Some(Value::Function(get)) = registry.get("get") else {
        return Ok(None);
    };
    let Value::Function(constructor) = get.call(&[Value::String(element.local_name.clone())])?
    else {
        return Ok(None);
    };
    if !document.mark_custom_element_upgraded(element.node) {
        return Ok(None);
    }
    object.set("__customElementUpgraded", Value::Boolean(true));
    if let Some(Value::Object(prototype)) = constructor.get_own_property("prototype") {
        object.set_prototype(Some(prototype));
    }
    if constructor.is_bytecode_function() {
        match constructor.construct(&[])? {
            Value::Object(instance) | Value::Array(instance) => {
                for key in instance.keys() {
                    if let Some(value) = instance.get_own(&key) {
                        object.set(key, value);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(Some(
        element
            .attributes
            .iter()
            .map(|attribute| (attribute.get_name().clone(), attribute.get_value().clone()))
            .collect(),
    ))
}

fn script_element_object(
    document: &Arc<Mutex<Document>>,
    registry: &Object,
    element: Element,
) -> JsResult<Object> {
    let (object, attributes, connected) = {
        let mut document_state = document.lock().unwrap();
        let object = if let Some(object) = document_state.script_elements.get(&element.node) {
            object.clone()
        } else {
            let object = element_object(element.clone(), document.clone());
            document_state
                .script_elements
                .insert(element.node, object.clone());
            object
        };
        let attributes = upgrade_element(&mut document_state, registry, &element, &object)?;
        let connected = attributes.is_some() && document_state.is_connected(element.node);
        (object, attributes, connected)
    };
    if let Some(attributes) = attributes {
        for (name, value) in attributes {
            invoke_attribute_changed_callback(&object, &name, None, Some(value));
        }
        if connected {
            invoke_lifecycle_callback(&object, "connectedCallback");
        }
    }
    Ok(object)
}

fn collect_elements(document: &HTMLDocument) -> Vec<Element> {
    fn collect(document: &HTMLDocument, parent: NodeID, elements: &mut Vec<Element>) {
        let Some(node) = document.node(parent) else {
            return;
        };

        for child in node.get_children() {
            if let Some(node) = document.node(*child) {
                if let NodeKind::Element(element) = node.get_kind() {
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
    fn window_document_is_the_same_object_as_the_document_global() {
        let document =
            Document::from_html_document(&parse_raw_html("<main id='app'></main>".to_owned()));
        let mut realm = Realm::new();
        register_document(&mut realm, document).unwrap();

        assert_eq!(
            realm.evaluate_script("window.document === document"),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script("window.document.getElementById('app') !== null"),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script("window.document = null; window.document === document"),
            Ok(Value::Boolean(true))
        );
    }

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
        assert_eq!(source.node(NodeID::new(2)).unwrap().get_parent(), None);
        assert_eq!(source.node(NodeID::new(3)).unwrap().get_parent(), None);
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
        assert_eq!(
            source.node(source.root).unwrap().get_children(),
            &vec![html_child]
        );
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
        assert_eq!(source.node(element).unwrap().get_children(), &Vec::new());
        assert_eq!(
            source.node(source.root).unwrap().get_children(),
            &vec![element, comment]
        );
        assert_eq!(
            source.node(comment).unwrap().get_parent(),
            Some(source.root)
        );
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
