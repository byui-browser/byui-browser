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
///
/// Wraps the HTML team's DOM; the real binding will hold a handle, not a copy.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Document {
    elements: Vec<Element>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// `document.getElementById(id)`.
    ///
    /// Currently only handles the empty document.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
    // signature however your crate's public API needs.
    pub fn get_element_by_id(&self, id: &str) -> Option<&Element> {
        if self.elements.is_empty() {
            return None;
        }
        todo!(
            "TODO(webapis): look up #{id} among {} elements",
            self.elements.len()
        )
    }
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
            realm.call_global("fetch", &[]).unwrap_err().to_string(),
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
    fn new_timer_queue_is_empty() {
        let mut queue = super::TimerQueue::new();
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.pop_next_due(), None);
    }
}
