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
use html::{HTMLDocument, NodeKind};
use js::{HostFunction, JsError, JsResult, Realm, Value};
use net::{Request, RequestController};

/// Destination for messages emitted by browser Web APIs.
pub trait ConsoleSink: Send + Sync {
    fn log(&self, message: &str);
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

    let body = String::from_utf8(response.body)
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
    pub node: NodeId,
    /// Value of the `id` attribute, if any.
    pub id_attr: Option<String>,
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
        Self::default()
    }

    /// Creates a script-visible document from the HTML document tree.
    pub fn from_html_document(document: &HTMLDocument) -> Self {
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
                            id_attr,
                        });
                    }
                    collect(document, *child, elements);
                }
            }
        }

        let mut elements = Vec::new();
        collect(document, document.root, &mut elements);
        Self {
            elements,
            source_document: Some(document.clone()),
        }
    }

    /// `document.getElementById(id)`.
    ///
    pub fn get_element_by_id(&self, id: &str) -> Option<&Element> {
        self.elements
            .iter()
            .find(|element| element.id_attr.as_deref() == Some(id))
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
        let Some(document) = &self.source_document else {
            return Ok(Vec::new());
        };
        let mut matches = Vec::new();

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
}

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
    use super::{ConsoleSink, fetch, print, register_fetch, register_print};
    use common::ids::NodeId;
    use html::parse_raw_html;
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
        let controller = RequestController::new(Config::default()).unwrap();

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
        let controller = std::sync::Arc::new(RequestController::new(Config::default()).unwrap());
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
    fn empty_document_has_no_element_by_id() {
        assert_eq!(super::Document::new().get_element_by_id("main"), None);
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
    fn new_timer_queue_is_empty() {
        let mut queue = super::TimerQueue::new();
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.pop_next_due(), None);
    }
}
