//! Bindings between JavaScript and DOM/platform APIs.
//!
//! **Owning team**: JS APIs (Web APIs) Team
//!
//! Window, Document, Element, Fetch, timers, events, and related APIs.
//! DOM mutations from JS must go through a controlled mutation interface
//! so style/layout invalidation stays correct.

#![forbid(unsafe_code)]

pub mod local_storage;

use std::sync::Arc;

use common::ids::NodeId;
use html::{Attribute, DOMTokenList, HTMLDocument, Namespace, NodeKind};
use js::{HostFunction, JsError, JsResult, Object, Realm, Value};
use net::{Request, RequestController};

/// Destination for messages emitted by browser Web APIs.
pub trait ConsoleSink: Send + Sync {
    fn log(&self, message: &str);
}

/// Destination for synchronous page alert requests.
///
/// The browser owns the concrete implementation. Keeping this boundary in
/// Web APIs lets the JavaScript engine and browser composition layer agree on
/// alert delivery without coupling the engine to a native window toolkit.
pub trait AlertSink: Send + Sync {
    /// Shows an alert containing `message` and returns after it is handled.
    fn alert(&self, message: &str);
}

/// Implements the JavaScript `alert(message)` Web API.
///
/// The optional first argument is converted using the JavaScript value's
/// standard display conversion. Additional arguments are ignored, matching
/// the browser API's single-message shape. The returned value is always
/// `undefined`.
pub fn alert(sink: &dyn AlertSink, arguments: &[Value]) -> JsResult<Value> {
    let message = arguments.first().map_or_else(String::new, Value::to_string);
    sink.alert(&message);
    Ok(Value::Undefined)
}

/// Installs the Web API global `alert` in a JavaScript realm.
pub fn register_alert(realm: &mut Realm, sink: Arc<dyn AlertSink>) -> JsResult<()> {
    let function: HostFunction = Arc::new(move |arguments| alert(sink.as_ref(), arguments));
    realm.register_global_function("alert", function)
}

/// JavaScript-facing `print` implementation.
///
/// The first slice intentionally has no arguments and emits a fixed message.
/// The output is injected so the renderer can route it to DevTools rather than
/// coupling Web APIs to process stdout.
pub fn print(console: &dyn ConsoleSink, arguments: &[Value]) -> JsResult<Value> {
    if !arguments.is_empty() {
        return Err(JsError::new("print() does not accept arguments"));
    }

    console.log("JS API Team Rocks!");
    Ok(Value::Undefined)
}

/// Installs the Web API global `print` in a JavaScript realm.
pub fn register_print(realm: &mut Realm, console: Arc<dyn ConsoleSink>) -> JsResult<()> {
    let function: HostFunction = Arc::new(move |arguments| print(console.as_ref(), arguments));
    realm.register_global_function("print", function)
}

/// JavaScript-facing `fetch` implementation.
///
/// The current JavaScript value model has no object or promise values, so this
/// first binding exposes the response body as a string. Network execution is
/// delegated to the networking crate's shared request controller.
pub fn fetch(controller: &RequestController, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(url)] = arguments else {
        return Err(JsError::new("fetch() expects exactly one URL string"));
    };

    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| JsError::new(format!("unable to start network runtime: {error}")))?;
    let response = runtime
        .block_on(controller.fetch(Request::get(url)))
        .map_err(|error| JsError::new(error.to_string()))?;

    let body = runtime
        .block_on(response.body.bytes())
        .map_err(|error| JsError::new(error.to_string()))?;
    let body = String::from_utf8(body)
        .map_err(|_| JsError::new("fetch() response body is not valid UTF-8"))?;
    Ok(Value::String(body))
}

/// Installs the Web API global `fetch` in a JavaScript realm.
pub fn register_fetch(realm: &mut Realm, controller: Arc<RequestController>) -> JsResult<()> {
    let function: HostFunction = Arc::new(move |arguments| fetch(controller.as_ref(), arguments));
    realm.register_global_function("fetch", function)
}

/// Script-visible view of a DOM element.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// Index of this element in its document's DOM arena.
    pub node: NodeId,
    /// The element's HTML local name, normalized to ASCII lowercase.
    pub local_name: String,
    /// Namespace of this element, used to apply the DOM's HTML name matching
    /// rules when attributes are read.
    pub namespace: Namespace,
    /// Value of the `id` attribute, if any.
    pub id_attr: Option<String>,
    /// Attributes in document order, including their original values.
    pub attributes: Vec<Attribute>,
}

impl Element {
    /// Returns the value of the first attribute matching `qualified_name`.
    ///
    /// This follows the DOM Standard's `Element.getAttribute()` algorithm:
    /// HTML elements in an HTML document match attribute names after ASCII
    /// lowercasing, while other namespaces use the name as provided. The
    /// returned value borrows this element's immutable attribute snapshot and
    /// is `None` when no matching attribute exists.
    pub fn get_attribute(&self, qualified_name: &str) -> Option<&str> {
        let name = match self.namespace {
            Namespace::Html => qualified_name.to_ascii_lowercase(),
            Namespace::Svg | Namespace::MathMl => qualified_name.to_owned(),
        };

        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
            .map(|attribute| attribute.value.as_str())
    }
}

fn element_object(element: Element) -> Object {
    let object = Object::new();
    let get_attribute_element = element;
    object.set_method(
        "getAttribute",
        Arc::new(move |arguments| get_attribute(&get_attribute_element, arguments)),
    );
    object
}

/// Implements JavaScript `Element.getAttribute(qualifiedName)`.
pub fn get_attribute(element: &Element, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(qualified_name)] = arguments else {
        return Err(JsError::new(
            "Element.getAttribute() expects one qualified name string",
        ));
    };

    Ok(match element.get_attribute(qualified_name) {
        Some(value) => Value::String(value.to_owned()),
        None => Value::Null,
    })
}

/// Implements JavaScript `document.getElementById(id)`.
pub fn get_element_by_id(document: &Document, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(id)] = arguments else {
        return Err(JsError::new(
            "document.getElementById() expects one id string",
        ));
    };

    Ok(match document.get_element_by_id(id) {
        Some(element) => Value::Object(element_object(element.clone())),
        None => Value::Null,
    })
}

/// Registers the read-only DOM methods supported by this crate on the global
/// JavaScript `document` object.
pub fn register_document(realm: &mut Realm, document: Document) -> JsResult<()> {
    let document_object = Object::new();
    let get_element_document = document;
    document_object.set_method(
        "getElementById",
        Arc::new(move |arguments| get_element_by_id(&get_element_document, arguments)),
    );
    realm.register_global_value("document", Value::Object(document_object))
}

/// Script-visible `document` object.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Document {
    elements: Vec<Element>,
    source_document: Option<HTMLDocument>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            source_document: Some(HTMLDocument::new()),
        }
    }

    /// Creates a script-visible document from the HTML document tree.
    pub fn from_html_document(document: &HTMLDocument) -> Self {
        Self {
            elements: collect_elements(document),
            source_document: Some(document.clone()),
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
            node: NodeId::new(node.index() as u32),
            local_name: element.name.clone(),
            namespace: element.namespace,
            id_attr: None,
            attributes: element.attributes.clone(),
        })
    }

    /// `document.getElementById(id)`.
    ///
    pub fn get_element_by_id(&self, id: &str) -> Option<&Element> {
        self.elements
            .iter()
            .find(|element| element.id_attr.as_deref() == Some(id))
    }

    /// Implements the DOM `Element.classList` getter for an element in this
    /// document. The returned live token list mutates the element's `class`
    /// attribute through the DOM Standard operations.
    pub fn class_list(&mut self, element: NodeId) -> JsResult<DOMTokenList<'_>> {
        self.source_document
            .get_or_insert_with(HTMLDocument::new)
            .class_list(html::NodeId(element.index() as usize))
            .map_err(|error| JsError::new(error.to_string()))
    }

    /// Removes an element node from this document's tree.
    ///
    /// The node and its descendants remain allocated and retain their stable
    /// IDs, but they are no longer reachable from the document root. Removing
    /// a detached node, the document root, or an invalid node ID is a no-op.
    pub fn remove(&mut self, node: NodeId) {
        let Some(document) = self.source_document.as_mut() else {
            return;
        };

        document.remove(html::NodeId(node.index() as usize));
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
    pub fn query_selector_all(&self, selector: &str) -> Result<Vec<NodeId>, SelectorError> {
        let selectors = parse_selector_list(selector)?;
        let mut matches = Vec::new();
        let Some(document) = &self.source_document else {
            return Ok(matches);
        };

        fn collect(
            document: &HTMLDocument,
            parent: html::NodeId,
            selectors: &[SimpleSelector],
            matches: &mut Vec<NodeId>,
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
                        matches.push(NodeId::new(child.index() as u32));
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
    pub fn append_child(&mut self, child: NodeId) -> Result<NodeId, DomException> {
        if self.source_document.is_none() {
            self.source_document = Some(HTMLDocument::new());
        }
        let source = self.source_document.as_ref().unwrap();
        let root = source.root;
        let child_id = html::NodeId(child.index() as usize);
        let child_node = source.node(child_id).ok_or(DomException::NotFoundError)?;

        if child_id == root || matches!(child_node.kind, NodeKind::Document) {
            return Err(DomException::HierarchyRequestError);
        }

        match &child_node.kind {
            NodeKind::Comment(_) | NodeKind::Element(_) | NodeKind::Doctype { .. } => {}
            NodeKind::Text(_) => return Err(DomException::HierarchyRequestError),
            NodeKind::Document => return Err(DomException::HierarchyRequestError),
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
        if let Some(parent) = document.nodes[child_id.0].parent {
            let siblings = &mut document.nodes[parent.0].children;
            let Some(position) = siblings.iter().position(|sibling| *sibling == child_id) else {
                return Err(DomException::NotFoundError);
            };
            siblings.remove(position);
        }
        document.nodes[child_id.0].parent = Some(root);
        document.nodes[root.0].children.push(child_id);
        self.elements = collect_elements(document);

        Ok(child)
    }
}

fn collect_elements(document: &HTMLDocument) -> Vec<Element> {
    fn collect(document: &HTMLDocument, parent: html::NodeId, elements: &mut Vec<Element>) {
        let Some(node) = document.node(parent) else {
            return;
        };

        for child in &node.children {
            if let Some(node) = document.node(*child) {
                if let NodeKind::Element(element) = &node.kind {
                    let id_attr = element
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == "id")
                        .map(|attribute| attribute.value.clone());
                    elements.push(Element {
                        node: NodeId::new(child.index() as u32),
                        local_name: element.name.clone(),
                        namespace: element.namespace,
                        id_attr,
                        attributes: element.attributes.clone(),
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

/// A DOM exception returned by an invalid `Document::append_child` operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomException {
    /// The insertion would violate the document's node hierarchy constraints.
    HierarchyRequestError,
    /// The node ID does not refer to a node in this document's arena.
    NotFoundError,
}

impl std::fmt::Display for DomException {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::HierarchyRequestError => "HierarchyRequestError",
            Self::NotFoundError => "NotFoundError",
        };
        formatter.write_str(name)
    }
}

impl std::error::Error for DomException {}

/// Error returned when a `query_selector_all` selector is invalid or unsupported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorError {
    /// Human-readable explanation of the selector error.
    pub message: String,
}

impl SelectorError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SelectorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SelectorError {}

#[derive(Debug)]
struct SimpleSelector {
    element_name: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl SimpleSelector {
    fn matches(&self, element: &html::ElementData) -> bool {
        if let Some(name) = &self.element_name
            && element.name != *name
        {
            return false;
        }
        if let Some(id) = &self.id
            && !element
                .attributes
                .iter()
                .any(|attribute| attribute.name == "id" && attribute.value == *id)
        {
            return false;
        }
        self.classes.iter().all(|class| {
            element.attributes.iter().any(|attribute| {
                attribute.name == "class"
                    && attribute
                        .value
                        .split_whitespace()
                        .any(|value| value == class)
            })
        })
    }
}

fn parse_selector_list(selector: &str) -> Result<Vec<SimpleSelector>, SelectorError> {
    selector
        .split(',')
        .map(str::trim)
        .map(parse_simple_selector)
        .collect()
}

fn parse_simple_selector(selector: &str) -> Result<SimpleSelector, SelectorError> {
    if selector.is_empty() || selector.chars().any(char::is_whitespace) {
        return Err(SelectorError::new("unsupported or empty selector"));
    }
    let mut parsed = SimpleSelector {
        element_name: None,
        id: None,
        classes: Vec::new(),
    };
    let mut remaining = selector;
    if let Some(first) = remaining.chars().next()
        && first != '#'
        && first != '.'
        && first != '*'
    {
        let end = remaining.find(['#', '.']).unwrap_or(remaining.len());
        let name = &remaining[..end];
        if !is_identifier(name) {
            return Err(SelectorError::new("invalid element selector"));
        }
        parsed.element_name = Some(name.to_ascii_lowercase());
        remaining = &remaining[end..];
    } else if remaining.starts_with('*') {
        remaining = &remaining[1..];
    }
    while !remaining.is_empty() {
        let marker = remaining.as_bytes()[0] as char;
        if marker != '#' && marker != '.' {
            return Err(SelectorError::new("invalid selector syntax"));
        }
        let value_end = remaining[1..]
            .find(['#', '.'])
            .map_or(remaining.len(), |index| index + 1);
        let value = &remaining[1..value_end];
        if !is_identifier(value) {
            return Err(SelectorError::new("invalid selector name"));
        }
        if marker == '#' {
            if parsed.id.replace(value.to_owned()).is_some() {
                return Err(SelectorError::new("selector has multiple IDs"));
            }
        } else {
            parsed.classes.push(value.to_owned());
        }
        remaining = &remaining[value_end..];
    }
    if parsed.element_name.is_none()
        && parsed.id.is_none()
        && parsed.classes.is_empty()
        && selector != "*"
    {
        return Err(SelectorError::new(
            "selector must contain a simple selector",
        ));
    }
    Ok(parsed)
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

/// Handle returned by `setTimeout`.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u32);

/// Pending `setTimeout` / `setInterval` callbacks, ordered by due time.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TimerQueue {
    pending: Vec<(TimerId, u64)>,
}

impl TimerQueue {
    /// An empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of timers not yet fired.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// `setTimeout(callback, delay_ms)`: schedules a callback.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
    // signature however your crate's public API needs.
    pub fn schedule(&mut self, delay_ms: u64) -> TimerId {
        todo!("TODO(webapis): schedule timer at +{delay_ms}ms")
    }

    /// Removes and returns the timer due soonest, if any.
    ///
    /// Currently only handles the empty queue.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn pop_next_due(&mut self) -> Option<TimerId> {
        if self.pending.is_empty() {
            return None;
        }
        todo!(
            "TODO(webapis): pop earliest of {} timers",
            self.pending.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AlertSink, ConsoleSink, DomException, alert, fetch, print, register_alert,
        register_document, register_fetch, register_print,
    };
    use common::ids::NodeId;
    use html::{Attribute, ElementData, HTMLDocument, Namespace, NodeKind, parse_raw_html};
    use js::{Realm, Value};
    use net::{Config, RequestController};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct TestConsole {
        messages: Mutex<Vec<String>>,
    }

    impl ConsoleSink for TestConsole {
        fn log(&self, message: &str) {
            self.messages.lock().unwrap().push(message.to_owned());
        }
    }

    #[derive(Default)]
    struct TestAlert {
        messages: Mutex<Vec<String>>,
    }

    impl AlertSink for TestAlert {
        fn alert(&self, message: &str) {
            self.messages.lock().unwrap().push(message.to_owned());
        }
    }

    #[test]
    fn alert_converts_the_first_argument_and_returns_undefined() {
        let sink = TestAlert::default();

        assert_eq!(
            alert(
                &sink,
                &[Value::Number(42.0), Value::String("ignored".into())]
            )
            .unwrap(),
            Value::Undefined
        );
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["42"]);
    }

    #[test]
    fn alert_without_an_argument_uses_an_empty_message() {
        let sink = TestAlert::default();

        alert(&sink, &[]).unwrap();

        assert_eq!(sink.messages.lock().unwrap().as_slice(), [""]);
    }

    #[test]
    fn print_sends_fixed_message_to_console() {
        let console = TestConsole::default();

        assert_eq!(print(&console, &[]).unwrap(), Value::Undefined);
        assert_eq!(
            console.messages.lock().unwrap().as_slice(),
            ["JS API Team Rocks!"]
        );
    }

    #[test]
    fn print_rejects_arguments() {
        let console = TestConsole::default();

        assert_eq!(
            print(&console, &[Value::String("hello".to_owned())])
                .unwrap_err()
                .to_string(),
            "print() does not accept arguments"
        );
    }

    #[test]
    fn fetch_requires_one_url_string() {
        let controller = RequestController::builder(Config::default())
            .build()
            .unwrap();

        assert_eq!(
            fetch(&controller, &[]).unwrap_err().to_string(),
            "fetch() expects exactly one URL string"
        );
        assert_eq!(
            fetch(&controller, &[Value::Boolean(true)])
                .unwrap_err()
                .to_string(),
            "fetch() expects exactly one URL string"
        );
    }

    #[test]
    fn registration_exposes_fetch_to_the_realm() {
        let controller = std::sync::Arc::new(
            RequestController::builder(Config::default())
                .build()
                .unwrap(),
        );
        let mut realm = Realm::new();
        register_fetch(&mut realm, controller).unwrap();

        assert_eq!(
            realm.evaluate_script("fetch()").unwrap_err().to_string(),
            "fetch() expects exactly one URL string"
        );
    }

    #[test]
    fn registration_exposes_print_to_the_realm() {
        let console = Arc::new(TestConsole::default());
        let mut realm = Realm::new();
        register_print(&mut realm, console.clone()).unwrap();

        assert_eq!(realm.evaluate_script("print()"), Ok(Value::Undefined));
        assert_eq!(
            console.messages.lock().unwrap().as_slice(),
            ["JS API Team Rocks!"]
        );
    }

    #[test]
    fn registration_exposes_alert_to_the_realm() {
        let sink = Arc::new(TestAlert::default());
        let mut realm = Realm::new();
        register_alert(&mut realm, sink.clone()).unwrap();

        assert_eq!(
            realm.evaluate_script("alert('hello')"),
            Ok(Value::Undefined)
        );
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["hello"]);
    }

    #[test]
    fn registration_exposes_element_get_attribute_to_javascript() {
        let html_document = parse_raw_html("<main id='app' data-state='ready'></main>".to_owned());
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            super::Document::from_html_document(&html_document),
        )
        .unwrap();

        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').getAttribute('DATA-STATE')")
                .unwrap(),
            Value::String("ready".to_owned())
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').getAttribute('missing')")
                .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn empty_document_has_no_element_by_id() {
        assert_eq!(super::Document::new().get_element_by_id("main"), None);
    }

    #[test]
    fn document_text_content_is_null_per_dom_standard() {
        let empty = super::Document::new();
        assert_eq!(empty.text_content(), None);

        let parsed = super::Document::from_html_document(&parse_raw_html(
            "<main>document text</main>".to_owned(),
        ));
        assert_eq!(parsed.text_content(), None);
    }

    #[test]
    fn document_get_element_by_id_uses_tree_order_and_exact_matching() {
        let html_document = parse_raw_html(
            "<section id='target'><span id='nested'></span></section><p id='nested'></p>"
                .to_owned(),
        );
        let document = super::Document::from_html_document(&html_document);

        let element = document
            .get_element_by_id("nested")
            .expect("nested element");
        assert_eq!(element.node.index(), 2);
        assert_eq!(document.get_element_by_id("NESTED"), None);
    }

    #[test]
    fn element_get_attribute_matches_html_names_case_insensitively() {
        let document = super::Document::from_html_document(&parse_raw_html(
            "<main id='app' data-state='ready' data-state='first'></main>".to_owned(),
        ));
        let element = document.get_element_by_id("app").expect("app element");

        assert_eq!(element.get_attribute("DATA-STATE"), Some("ready"));
        assert_eq!(element.get_attribute("missing"), None);
    }

    #[test]
    fn created_element_get_attribute_returns_none_without_attributes() {
        let mut document = super::Document::new();
        let element = document.create_element("custom-widget").unwrap();

        assert_eq!(element.get_attribute("data-state"), None);
    }

    #[test]
    fn create_element_lowercases_and_returns_a_detached_html_element() {
        let mut document = super::Document::new();

        let element = document.create_element("CuStOm-Widget").unwrap();

        assert_eq!(element.local_name, "custom-widget");
        assert_eq!(element.node.index(), 1);
        assert_eq!(element.id_attr, None);
    }

    #[test]
    fn create_element_rejects_invalid_local_names() {
        let mut document = super::Document::new();

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
        let mut document = super::Document::from_html_document(&parse_raw_html(
            "<div id='existing'></div>".into(),
        ));
        let first = document.create_element("DIV").unwrap();
        let second = document.create_element("span").unwrap();
        assert_eq!(first.node, NodeId::new(2));
        assert_eq!(second.node, NodeId::new(3));
        assert_eq!(
            document.query_selector_all("*").unwrap(),
            vec![NodeId::new(1)]
        );
        assert_eq!(
            document.get_element_by_id("existing").unwrap().node,
            NodeId::new(1)
        );
        let source = document.source_document.as_ref().unwrap();
        assert_eq!(source.node(html::NodeId(2)).unwrap().parent, None);
        assert_eq!(source.node(html::NodeId(3)).unwrap().parent, None);
    }

    #[test]
    fn document_remove_detaches_the_node_from_queries() {
        let html_document = parse_raw_html(
            "<section><span id='target'></span></section><p id='other'></p>".to_owned(),
        );
        let mut document = super::Document::from_html_document(&html_document);
        let section = NodeId::new(1);

        document.remove(section);

        assert_eq!(document.get_element_by_id("target"), None);
        assert_eq!(
            document.get_element_by_id("other").unwrap().node,
            NodeId::new(3)
        );
        assert_eq!(document.query_selector_all("span").unwrap(), Vec::new());
    }

    #[test]
    fn document_remove_of_detached_or_root_nodes_is_a_no_op() {
        let html_document = parse_raw_html("<div id='target'></div>".to_owned());
        let mut document = super::Document::from_html_document(&html_document);

        document.remove(NodeId::new(99));
        document.remove(NodeId::new(0));

        assert!(document.get_element_by_id("target").is_some());
    }

    #[test]
    fn document_query_selector_all_returns_matching_descendants_in_tree_order() {
        let html_document = parse_raw_html(
            "<main><article class='post target'><span class='target'></span></article><p class='target'></p></main>"
                .to_owned(),
        );
        let document = super::Document::from_html_document(&html_document);

        assert_eq!(
            document.query_selector_all(".target, article").unwrap(),
            vec![NodeId::new(2), NodeId::new(3), NodeId::new(4)]
        );
        assert_eq!(
            document.query_selector_all("*").unwrap(),
            vec![
                NodeId::new(1),
                NodeId::new(2),
                NodeId::new(3),
                NodeId::new(4)
            ]
        );
    }

    #[test]
    fn document_query_selector_all_rejects_invalid_selectors() {
        let document = super::Document::from_html_document(&parse_raw_html("<div></div>".into()));

        assert!(document.query_selector_all("").is_err());
        assert!(document.query_selector_all("div,").is_err());
        assert_eq!(document.query_selector_all(".missing").unwrap(), Vec::new());
    }

    #[test]
    fn document_append_child_adds_a_detached_element_and_returns_its_id() {
        let mut html_document = HTMLDocument::new();
        let html_child = html_document.push_node(
            NodeKind::Element(ElementData {
                name: "main".into(),
                namespace: Namespace::Html,
                attributes: vec![Attribute {
                    name: "id".into(),
                    value: "appended".into(),
                }],
            }),
            None,
        );
        let mut document = super::Document::from_html_document(&html_document);
        let child = NodeId::new(html_child.index() as u32);

        assert_eq!(document.append_child(child), Ok(child));
        let source = document.source_document.as_ref().unwrap();
        assert_eq!(source.node(source.root).unwrap().children, vec![html_child]);
        assert_eq!(document.get_element_by_id("appended").unwrap().node, child);
    }

    #[test]
    fn document_append_child_moves_a_child_from_its_old_parent() {
        let mut html_document = HTMLDocument::new();
        let element = html_document.push_node(
            NodeKind::Element(ElementData {
                name: "main".into(),
                namespace: Namespace::Html,
                attributes: Vec::new(),
            }),
            None,
        );
        let comment = html_document.push_node(NodeKind::Comment("move me".into()), None);
        html_document.append_child(html_document.root, element);
        html_document.append_child(element, comment);
        let mut document = super::Document::from_html_document(&html_document);
        let comment_id = NodeId::new(comment.index() as u32);

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
            NodeKind::Element(ElementData {
                name: "main".into(),
                namespace: Namespace::Html,
                attributes: Vec::new(),
            }),
            None,
        );
        let second_element = html_document.push_node(
            NodeKind::Element(ElementData {
                name: "aside".into(),
                namespace: Namespace::Html,
                attributes: Vec::new(),
            }),
            None,
        );
        html_document.append_child(html_document.root, existing_element);
        let mut document = super::Document::from_html_document(&html_document);
        let before = document.source_document.clone();

        assert_eq!(
            document.append_child(NodeId::new(second_element.index() as u32)),
            Err(DomException::HierarchyRequestError)
        );
        assert_eq!(document.source_document, before);
    }

    #[test]
    fn document_append_child_reports_unknown_node_ids() {
        let mut document = super::Document::new();

        assert_eq!(
            document.append_child(NodeId::new(99)),
            Err(DomException::NotFoundError)
        );
    }

    #[test]
    fn new_timer_queue_is_empty() {
        let mut queue = super::TimerQueue::new();
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.pop_next_due(), None);
    }
}
