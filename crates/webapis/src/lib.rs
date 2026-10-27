//! Bindings between JavaScript and DOM/platform APIs.
//!
//! **Owning team**: JS APIs (Web APIs) Team
//!
//! Window, Document, Element, Fetch, timers, events, and related APIs.
//! DOM mutations from JS must go through a controlled mutation interface
//! so style/layout invalidation stays correct.

#![forbid(unsafe_code)]

pub mod local_storage;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use html::{Attribute, DOMTokenList, HTMLDocument, Namespace, NodeID, NodeKind};
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
    pub node: NodeID,
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

    /// Sets an attribute according to the DOM Standard's name validation and
    /// namespace normalization rules.
    pub fn set_attribute(&mut self, qualified_name: &str, value: &str) -> JsResult<()> {
        let name = normalize_attribute_name(self.namespace, qualified_name)?;
        if let Some(attribute) = self
            .attributes
            .iter_mut()
            .find(|attribute| attribute.name == name)
        {
            attribute.value = value.to_owned();
        } else {
            self.attributes.push(Attribute {
                name,
                value: value.to_owned(),
            });
        }
        self.id_attr = self
            .attributes
            .iter()
            .find(|attribute| attribute.name == "id")
            .map(|attribute| attribute.value.clone());
        Ok(())
    }
}

fn element_object(element: Element, document: Arc<Mutex<Document>>) -> Object {
    let object = Object::new();
    let node = element.node;
    let style_value = element
        .get_attribute("style")
        .map_or_else(String::new, str::to_owned);
    let style = Object::new();
    style.set("cssText", Value::String(style_value.clone()));
    for (name, value) in parse_inline_style(&style_value) {
        style.set(css_property_to_js(&name), Value::String(value));
    }
    object.set("style", Value::Object(style));
    let get_attribute_document = document.clone();
    object.set_method(
        "getAttribute",
        Arc::new(move |arguments| {
            let document = get_attribute_document.lock().unwrap();
            let Some(element) = document.element_by_node(node) else {
                return Ok(Value::Null);
            };
            get_attribute(element, arguments)
        }),
    );
    let set_attribute_document = document.clone();
    object.set_method(
        "setAttribute",
        Arc::new(move |arguments| {
            let [Value::String(qualified_name), Value::String(value)] = arguments else {
                return Err(JsError::new(
                    "Element.setAttribute() expects a qualified name and value string",
                ));
            };
            set_attribute_document
                .lock()
                .unwrap()
                .set_attribute(node, qualified_name, value)
        }),
    );
    let add_document = document.clone();
    let add_node = element.node;
    object.set_method(
        "addEventListener",
        Arc::new(move |arguments| {
            let document = add_document.lock().unwrap();
            add_event_listener_at(&document, Some(add_node), arguments)
        }),
    );
    let remove_document = document.clone();
    let remove_node = element.node;
    object.set_method(
        "removeEventListener",
        Arc::new(move |arguments| {
            let document = remove_document.lock().unwrap();
            remove_event_listener_at(&document, Some(remove_node), arguments)
        }),
    );
    let dispatch_document = document.clone();
    let dispatch_node = element.node;
    let dispatch_target = object.clone();
    object.set_method(
        "dispatchEvent",
        Arc::new(move |arguments| {
            let document = dispatch_document.lock().unwrap();
            dispatch_event_at(&document, Some(dispatch_node), &dispatch_target, arguments)
        }),
    );
    object
}

/// Reads declarations from an HTML `style` attribute.
fn parse_inline_style(style: &str) -> Vec<(String, String)> {
    let mut declarations = Vec::new();
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();
        if property.is_empty() || value.is_empty() {
            continue;
        }
        if let Some(existing) = declarations.iter_mut().find(|(name, _)| name == &property) {
            existing.1 = value.to_owned();
        } else {
            declarations.push((property, value.to_owned()));
        }
    }
    declarations
}

fn css_property_to_js(property: &str) -> String {
    let mut result = String::with_capacity(property.len());
    let mut uppercase_next = false;
    for character in property.chars() {
        if character == '-' {
            uppercase_next = true;
        } else if uppercase_next {
            result.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            result.push(character);
        }
    }
    result
}

fn normalize_attribute_name(namespace: Namespace, qualified_name: &str) -> JsResult<String> {
    if qualified_name.is_empty()
        || qualified_name.chars().any(|character| {
            matches!(
                character,
                '\t' | '\n' | '\x0c' | '\r' | ' ' | '\0' | '/' | '=' | '>'
            )
        })
    {
        return Err(JsError::new("InvalidCharacterError"));
    }

    Ok(match namespace {
        Namespace::Html => qualified_name.to_ascii_lowercase(),
        Namespace::Svg | Namespace::MathMl => qualified_name.to_owned(),
    })
}

#[derive(Debug, Clone)]
struct EventListener {
    event_type: String,
    callback: Value,
    capture: bool,
    passive: Option<bool>,
    once: bool,
    signal: Option<Object>,
}

#[derive(Debug, Default)]
struct EventTarget {
    listeners: Vec<EventListener>,
}

type EventTargetRef = Arc<Mutex<EventTarget>>;

impl EventTarget {
    fn listeners(&self) -> &[EventListener] {
        &self.listeners
    }

    fn listeners_mut(&mut self) -> &mut Vec<EventListener> {
        &mut self.listeners
    }
}

fn lock_event_target(target: &Mutex<EventTarget>) -> MutexGuard<'_, EventTarget> {
    target.lock().unwrap_or_else(PoisonError::into_inner)
}

fn javascript_truthy(value: &Value) -> bool {
    match value {
        Value::Undefined | Value::Null => false,
        Value::Boolean(value) => *value,
        Value::Number(value) => *value != 0.0 && !value.is_nan(),
        Value::String(value) => !value.is_empty(),
        Value::Function(_) | Value::Object(_) => true,
    }
}

fn event_listener_options(options: Option<&Value>) -> (bool, Option<bool>, bool, Option<Object>) {
    match options {
        Some(Value::Boolean(capture)) => (*capture, None, false, None),
        Some(Value::Object(options)) => {
            let capture = options
                .get("capture")
                .is_some_and(|value| javascript_truthy(&value));
            let passive = options
                .get("passive")
                .map(|value| javascript_truthy(&value));
            let once = options
                .get("once")
                .is_some_and(|value| javascript_truthy(&value));
            let signal = match options.get("signal") {
                Some(Value::Object(signal)) => Some(signal),
                _ => None,
            };
            (capture, passive, once, signal)
        }
        _ => (false, None, false, None),
    }
}

fn event_object(event_type: String, bubbles: bool, cancelable: bool) -> Object {
    let event = Object::new();
    event.set("type", Value::String(event_type));
    event.set("bubbles", Value::Boolean(bubbles));
    event.set("cancelable", Value::Boolean(cancelable));
    event.set("defaultPrevented", Value::Boolean(false));
    event.set("target", Value::Null);
    event.set("currentTarget", Value::Null);
    event.set("eventPhase", Value::Number(0.0));
    event.set("__stopPropagation", Value::Boolean(false));
    event.set("__stopImmediatePropagation", Value::Boolean(false));
    event.set("__passiveListener", Value::Boolean(false));

    let prevent_event = event.clone();
    event.set_method(
        "preventDefault",
        Arc::new(move |_| {
            let cancelable = matches!(prevent_event.get("cancelable"), Some(Value::Boolean(true)));
            let passive = matches!(
                prevent_event.get("__passiveListener"),
                Some(Value::Boolean(true))
            );
            if cancelable && !passive {
                prevent_event.set("defaultPrevented", Value::Boolean(true));
            }
            Ok(Value::Undefined)
        }),
    );

    let stop_event = event.clone();
    event.set_method(
        "stopPropagation",
        Arc::new(move |_| {
            stop_event.set("__stopPropagation", Value::Boolean(true));
            Ok(Value::Undefined)
        }),
    );

    let immediate_event = event.clone();
    event.set_method(
        "stopImmediatePropagation",
        Arc::new(move |_| {
            immediate_event.set("__stopPropagation", Value::Boolean(true));
            immediate_event.set("__stopImmediatePropagation", Value::Boolean(true));
            Ok(Value::Undefined)
        }),
    );
    event
}

/// Implements the JavaScript `AbortController()` constructor used to cancel
/// event listeners registered with its signal.
pub fn abort_controller_constructor(_arguments: &[Value]) -> JsResult<Value> {
    let controller = Object::new();
    let signal = Object::new();
    signal.set("aborted", Value::Boolean(false));
    let abort_signal = signal.clone();
    controller.set("signal", Value::Object(signal));
    controller.set_method(
        "abort",
        Arc::new(move |_| {
            abort_signal.set("aborted", Value::Boolean(true));
            Ok(Value::Undefined)
        }),
    );
    Ok(Value::Object(controller))
}

/// Implements the JavaScript `Event(type, options)` constructor supported by
/// this runtime. The JavaScript parser does not currently support `new`, so
/// the constructor is callable directly.
pub fn event_constructor(arguments: &[Value]) -> JsResult<Value> {
    let Some(event_type) = arguments.first() else {
        return Err(JsError::new("Event() expects an event type"));
    };
    let (bubbles, cancelable) = match arguments.get(1) {
        Some(Value::Object(options)) => (
            options
                .get("bubbles")
                .is_some_and(|value| javascript_truthy(&value)),
            options
                .get("cancelable")
                .is_some_and(|value| javascript_truthy(&value)),
        ),
        _ => (false, false),
    };
    Ok(Value::Object(event_object(
        event_type.to_string(),
        bubbles,
        cancelable,
    )))
}

/// Implements the DOM Standard's `EventTarget.addEventListener()` listener
/// registration algorithm for a document or exposed element target.
///
/// Listener records are retained with their `capture`, `passive`, `once`, and
/// `signal` options, and duplicate `(type, callback, capture)` registrations
/// are ignored. Registered callbacks are invoked by `dispatchEvent`, and
/// listeners associated with an aborted signal are removed before dispatch.
pub fn add_event_listener(document: &Document, arguments: &[Value]) -> JsResult<Value> {
    add_event_listener_at(document, None, arguments)
}

fn event_target_for(document: &Document, node: Option<NodeID>) -> EventTargetRef {
    match node {
        None => document.event_target.clone(),
        Some(node) => {
            let mut targets = document
                .event_targets
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            targets
                .entry(node.get() as usize)
                .or_insert_with(|| Arc::new(Mutex::new(EventTarget::default())))
                .clone()
        }
    }
}

fn add_event_listener_at(
    document: &Document,
    node: Option<NodeID>,
    arguments: &[Value],
) -> JsResult<Value> {
    let [event_type, callback, options @ ..] = arguments else {
        return Err(JsError::new(
            "document.addEventListener() expects a type and callback",
        ));
    };

    let event_type = event_type.to_string();
    let is_null_callback = matches!(callback, Value::Null | Value::Undefined);
    if !is_null_callback {
        match callback {
            Value::Function(_) => {}
            Value::Object(callback)
                if matches!(callback.get("handleEvent"), Some(Value::Function(_))) => {}
            Value::Object(_) => {
                return Err(JsError::new(
                    "document.addEventListener() callback object must define handleEvent",
                ));
            }
            _ => {
                return Err(JsError::new(
                    "document.addEventListener() callback must be a function, object, or null",
                ));
            }
        }
    }

    let (capture, passive, once, signal) = event_listener_options(options.first());
    let passive = passive.or_else(|| {
        matches!(
            event_type.as_str(),
            "touchstart" | "touchmove" | "wheel" | "mousewheel"
        )
        .then_some(true)
    });
    if signal
        .as_ref()
        .and_then(|signal| signal.get("aborted"))
        .is_some_and(|aborted| javascript_truthy(&aborted))
    {
        return Ok(Value::Undefined);
    }
    if is_null_callback {
        return Ok(Value::Undefined);
    }

    let target_ref = event_target_for(document, node);
    let mut target = lock_event_target(&target_ref);
    if target.listeners().iter().any(|listener| {
        listener.event_type == event_type
            && listener.capture == capture
            && listener.callback == *callback
    }) {
        return Ok(Value::Undefined);
    }

    target.listeners_mut().push(EventListener {
        event_type,
        callback: callback.clone(),
        capture,
        passive,
        once,
        signal,
    });
    Ok(Value::Undefined)
}

/// Implements `EventTarget.removeEventListener()` for the document target.
///
/// Removal matches the DOM listener key of `(type, callback, capture)`;
/// `passive`, `once`, and `signal` do not participate in the match.
pub fn remove_event_listener(document: &Document, arguments: &[Value]) -> JsResult<Value> {
    remove_event_listener_at(document, None, arguments)
}

fn remove_event_listener_at(
    document: &Document,
    node: Option<NodeID>,
    arguments: &[Value],
) -> JsResult<Value> {
    let [event_type, callback, options @ ..] = arguments else {
        return Err(JsError::new(
            "document.removeEventListener() expects a type and callback",
        ));
    };
    let event_type = event_type.to_string();
    if matches!(callback, Value::Null | Value::Undefined) {
        return Ok(Value::Undefined);
    }
    if !matches!(callback, Value::Function(_) | Value::Object(_)) {
        return Err(JsError::new(
            "document.removeEventListener() callback must be a function, object, or null",
        ));
    }
    let (capture, _, _, _) = event_listener_options(options.first());
    let target_ref = event_target_for(document, node);
    lock_event_target(&target_ref)
        .listeners_mut()
        .retain(|listener| {
            !(listener.event_type == event_type
                && listener.capture == capture
                && listener.callback == *callback)
        });
    Ok(Value::Undefined)
}

fn invoke_event_listener(callback: &Value, event: &Object) -> JsResult<Value> {
    match callback {
        Value::Function(callback) => callback.call(&[Value::Object(event.clone())]),
        Value::Object(callback) => match callback.get("handleEvent") {
            Some(Value::Function(handle_event)) => {
                handle_event.call(&[Value::Object(event.clone())])
            }
            _ => Err(JsError::new(
                "event listener object has no handleEvent method",
            )),
        },
        _ => Err(JsError::new("invalid event listener callback")),
    }
}

fn dispatch_event_with_target(
    document: &Document,
    target_object: &Object,
    arguments: &[Value],
) -> JsResult<Value> {
    dispatch_event_at(document, None, target_object, arguments)
}

fn invoke_listeners(
    target: &EventTargetRef,
    event: &Object,
    event_type: &str,
    current_target: &Object,
    capture: bool,
    phase: f64,
) {
    lock_event_target(target)
        .listeners_mut()
        .retain(|listener| {
            !listener
                .signal
                .as_ref()
                .and_then(|signal| signal.get("aborted"))
                .is_some_and(|aborted| javascript_truthy(&aborted))
        });
    event.set("currentTarget", Value::Object(current_target.clone()));
    event.set("eventPhase", Value::Number(phase));
    let listeners = lock_event_target(target)
        .listeners()
        .iter()
        .filter(|listener| listener.event_type == event_type && listener.capture == capture)
        .cloned()
        .collect::<Vec<_>>();
    for listener in listeners {
        if matches!(
            event.get("__stopImmediatePropagation"),
            Some(Value::Boolean(true))
        ) {
            break;
        }
        if listener.once {
            lock_event_target(target).listeners_mut().retain(|current| {
                !(current.event_type == listener.event_type
                    && current.capture == listener.capture
                    && current.callback == listener.callback)
            });
        }
        event.set(
            "__passiveListener",
            Value::Boolean(listener.passive.unwrap_or(false)),
        );
        let _ = invoke_event_listener(&listener.callback, event);
        event.set("__passiveListener", Value::Boolean(false));
    }
}

fn element_for_node(document: &Document, node: NodeID) -> Option<Element> {
    document
        .elements
        .iter()
        .find(|element| element.node == node)
        .cloned()
}

fn dispatch_event_at(
    document: &Document,
    node: Option<NodeID>,
    target_object: &Object,
    arguments: &[Value],
) -> JsResult<Value> {
    let [Value::Object(event)] = arguments else {
        return Err(JsError::new(
            "document.dispatchEvent() expects one Event object",
        ));
    };
    let Some(Value::String(event_type)) = event.get("type") else {
        return Err(JsError::new(
            "document.dispatchEvent() expects an Event object",
        ));
    };

    event.set("target", Value::Object(target_object.clone()));
    event.set("currentTarget", Value::Object(target_object.clone()));
    event.set("eventPhase", Value::Number(2.0));
    event.set("__stopPropagation", Value::Boolean(false));
    event.set("__stopImmediatePropagation", Value::Boolean(false));

    let mut path = Vec::new();
    let mut current = node;
    while let Some(current_node) = current {
        path.push(current_node);
        current = document
            .source_document
            .as_ref()
            .and_then(|source| source.node(current_node).and_then(|n| n.parent))
            .map(|parent| parent);
    }
    let document_target = document.event_target.clone();
    if node.is_some() {
        invoke_listeners(
            &document_target,
            event,
            &event_type,
            &Object::new(),
            true,
            1.0,
        );
        for ancestor in path.iter().rev().skip(1).take(path.len().saturating_sub(2)) {
            if matches!(event.get("__stopPropagation"), Some(Value::Boolean(true))) {
                break;
            }
            if let Some(element) = element_for_node(document, *ancestor) {
                let target = event_target_for(document, Some(*ancestor));
                let object = element_object(element, Arc::new(Mutex::new(document.clone())));
                invoke_listeners(&target, event, &event_type, &object, true, 1.0);
            }
        }
        if let Some(target_node) = node {
            let target = event_target_for(document, Some(target_node));
            invoke_listeners(&target, event, &event_type, target_object, true, 2.0);
            if !matches!(
                event.get("__stopImmediatePropagation"),
                Some(Value::Boolean(true))
            ) {
                invoke_listeners(&target, event, &event_type, target_object, false, 2.0);
            }
        }
        if matches!(event.get("bubbles"), Some(Value::Boolean(true)))
            && !matches!(event.get("__stopPropagation"), Some(Value::Boolean(true)))
        {
            for ancestor in &path[1..] {
                if matches!(event.get("__stopPropagation"), Some(Value::Boolean(true))) {
                    break;
                }
                if let Some(element) = element_for_node(document, *ancestor) {
                    let target = event_target_for(document, Some(*ancestor));
                    let object = element_object(element, Arc::new(Mutex::new(document.clone())));
                    invoke_listeners(&target, event, &event_type, &object, false, 3.0);
                }
            }
            invoke_listeners(
                &document_target,
                event,
                &event_type,
                &Object::new(),
                false,
                3.0,
            );
        }
    } else {
        invoke_listeners(
            &document_target,
            event,
            &event_type,
            target_object,
            true,
            2.0,
        );
        if !matches!(
            event.get("__stopImmediatePropagation"),
            Some(Value::Boolean(true))
        ) {
            invoke_listeners(
                &document_target,
                event,
                &event_type,
                target_object,
                false,
                2.0,
            );
        }
    }

    event.set("eventPhase", Value::Number(0.0));
    event.set("currentTarget", Value::Null);
    Ok(Value::Boolean(!matches!(
        event.get("defaultPrevented"),
        Some(Value::Boolean(true))
    )))
}

/// Implements `EventTarget.dispatchEvent()` for the document target.
pub fn dispatch_event(document: &Document, arguments: &[Value]) -> JsResult<Value> {
    dispatch_event_with_target(document, &Object::new(), arguments)
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
    elements: Vec<Element>,
    source_document: Option<HTMLDocument>,
    event_target: Arc<Mutex<EventTarget>>,
    event_targets: Arc<Mutex<HashMap<usize, EventTargetRef>>>,
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

    fn element_by_node(&self, node: NodeID) -> Option<&Element> {
        self.elements.iter().find(|element| element.node == node)
    }

    fn set_attribute(
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
                        .attributes
                        .iter()
                        .find(|attribute| attribute.name == "id")
                        .map(|attribute| attribute.value.clone());
                    elements.push(Element {
                        node: *child,
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
        AlertSink, ConsoleSink, DomException, add_event_listener, alert, dispatch_event,
        event_constructor, fetch, lock_event_target, print, register_alert, register_document,
        register_fetch, register_print, remove_event_listener,
    };
    use html::{Attribute, ElementData, HTMLDocument, Namespace, NodeID, NodeKind, parse_raw_html};
    use js::{JsError, Realm, Value};
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
    fn registration_exposes_document_add_event_listener_to_javascript() {
        let html_document = parse_raw_html("<main></main>".to_owned());
        let document = super::Document::from_html_document(&html_document);
        let target = document.event_target.clone();
        let mut realm = Realm::new();
        register_document(&mut realm, document).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "function callback(event) {} document.addEventListener('ready', callback)",
            ),
            Ok(Value::Undefined)
        );
        assert_eq!(lock_event_target(&target).listeners().len(), 1);
    }

    #[test]
    fn javascript_event_dispatch_invokes_callbacks_and_remove_works() {
        let html_document = parse_raw_html("<main></main>".to_owned());
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            super::Document::from_html_document(&html_document),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var count = 0\n\
                 function callback(event) { count = count + 1 }\n\
                 document.addEventListener('ready', callback)\n\
                 document.dispatchEvent(Event('ready'))\n\
                 document.dispatchEvent(Event('ready'))\n\
                 count",
            ),
            Ok(Value::Number(2.0))
        );
    }

    #[test]
    fn element_style_exposes_inline_css2_declarations_as_camel_case_properties() {
        let html_document = parse_raw_html(
            "<main id='app' style='color: red; background-color: blue; color: green; broken'></main>"
                .to_owned(),
        );
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            super::Document::from_html_document(&html_document),
        )
        .unwrap();

        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').style.color")
                .unwrap(),
            Value::String("green".to_owned())
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').style.backgroundColor")
                .unwrap(),
            Value::String("blue".to_owned())
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').style.cssText")
                .unwrap(),
            Value::String("color: red; background-color: blue; color: green; broken".to_owned())
        );
    }

    #[test]
    fn registration_exposes_element_set_attribute_to_javascript() {
        let html_document = parse_raw_html("<main id='app' data-state='ready'></main>".to_owned());
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            super::Document::from_html_document(&html_document),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script(
                "document.getElementById('app').setAttribute('DATA-STATE', 'updated'); document.getElementById('app').getAttribute('data-state')",
            ),
            Ok(Value::String("updated".to_owned()))
        );
        assert_eq!(
            realm.evaluate_script(
                "document.getElementById('app').setAttribute('data-added', 'yes')",
            ),
            Ok(Value::Undefined)
        );
        assert_eq!(
            realm
                .evaluate_script("document.getElementById('app').getAttribute('DATA-ADDED')")
                .unwrap(),
            Value::String("yes".to_owned())
        );
    }

    #[test]
    fn element_events_capture_target_and_bubble_to_document_ancestors() {
        let html_document =
            parse_raw_html("<main id='container'><button id='button'></button></main>".to_owned());
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            super::Document::from_html_document(&html_document),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var order = ''\n\
                 function parentCapture(event) { order = order + 'C' }\n\
                 function target(event) { order = order + 'T' }\n\
                 function parentBubble(event) { order = order + 'B' }\n\
                 document.getElementById('container').addEventListener('click', parentCapture, true)\n\
                 document.getElementById('container').addEventListener('click', parentBubble)\n\
                 document.getElementById('button').addEventListener('click', target)\n\
                 var event = Event('click')\n\
                 event.bubbles = true\n\
                 document.getElementById('button').dispatchEvent(event)\n\
                 order",
            ),
            Ok(Value::String("CTB".into()))
        );
    }

    #[test]
    fn abort_controller_removes_a_signal_bound_listener() {
        let html_document = parse_raw_html("<main></main>".to_owned());
        let mut realm = Realm::new();
        register_document(
            &mut realm,
            super::Document::from_html_document(&html_document),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script(
                "var count = 0\n\
                 var controller = AbortController()\n\
                 var options = Event('options')\n\
                 options.signal = controller.signal\n\
                 function callback(event) { count = count + 1 }\n\
                 document.addEventListener('ready', callback, options)\n\
                 controller.abort()\n\
                 document.dispatchEvent(Event('ready'))\n\
                 count",
            ),
            Ok(Value::Number(0.0))
        );
    }

    #[test]
    fn dispatch_event_returns_false_when_a_callback_prevents_default() {
        let document = super::Document::new();
        let callback = Value::Function(js::Function::from_host(
            "callback",
            Arc::new(|arguments| {
                let [Value::Object(event)] = arguments else {
                    return Err(JsError::new("event callback argument missing"));
                };
                let Some(Value::Function(prevent_default)) = event.get("preventDefault") else {
                    return Err(JsError::new("event.preventDefault() missing"));
                };
                prevent_default.call(&[])?;
                Ok(Value::Undefined)
            }),
        ));
        add_event_listener(
            &document,
            &[Value::String("ready".into()), callback.clone()],
        )
        .unwrap();
        let options = js::Object::new();
        options.set("cancelable", Value::Boolean(true));
        let event =
            event_constructor(&[Value::String("ready".into()), Value::Object(options)]).unwrap();
        assert_eq!(
            dispatch_event(&document, &[event]),
            Ok(Value::Boolean(false))
        );
        remove_event_listener(&document, &[Value::String("ready".into()), callback]).unwrap();
    }

    #[test]
    fn add_event_listener_follows_duplicate_and_option_rules() {
        let document = super::Document::new();
        let callback = Value::Function(js::Function::from_host(
            "callback",
            Arc::new(|_| Ok(Value::Undefined)),
        ));
        let options = js::Object::new();
        options.set("capture", Value::Boolean(true));
        options.set("once", Value::Boolean(true));
        options.set("passive", Value::Boolean(true));

        add_event_listener(
            &document,
            &[
                Value::String("ready".into()),
                callback.clone(),
                Value::Object(options),
            ],
        )
        .unwrap();
        add_event_listener(
            &document,
            &[
                Value::String("ready".into()),
                callback.clone(),
                Value::Boolean(true),
            ],
        )
        .unwrap();

        {
            let target = lock_event_target(&document.event_target);
            assert_eq!(target.listeners().len(), 1);
            assert!(target.listeners()[0].capture);
            assert_eq!(target.listeners()[0].passive, Some(true));
            assert!(target.listeners()[0].once);
        }

        let touch_callback = Value::Function(js::Function::from_host(
            "touch_callback",
            Arc::new(|_| Ok(Value::Undefined)),
        ));
        add_event_listener(
            &document,
            &[Value::String("touchstart".into()), touch_callback],
        )
        .unwrap();
        assert_eq!(
            lock_event_target(&document.event_target).listeners()[1].passive,
            Some(true)
        );
    }

    #[test]
    fn add_event_listener_ignores_null_and_aborted_callbacks() {
        let document = super::Document::new();
        add_event_listener(&document, &[Value::String("ready".into()), Value::Null]).unwrap();

        let signal = js::Object::new();
        signal.set("aborted", Value::Boolean(true));
        let callback = Value::Function(js::Function::from_host(
            "callback",
            Arc::new(|_| Ok(Value::Undefined)),
        ));
        let options = js::Object::new();
        options.set("signal", Value::Object(signal));
        add_event_listener(
            &document,
            &[
                Value::String("ready".into()),
                callback,
                Value::Object(options),
            ],
        )
        .unwrap();

        assert!(
            lock_event_target(&document.event_target)
                .listeners()
                .is_empty()
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
        assert_eq!(element.node.get(), 2);
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
    fn element_set_attribute_rejects_invalid_names() {
        let mut element = super::Element {
            node: NodeID::new(1),
            local_name: "main".into(),
            namespace: Namespace::Html,
            id_attr: None,
            attributes: Vec::new(),
        };
        assert_eq!(
            element
                .set_attribute("bad name", "value")
                .unwrap_err()
                .to_string(),
            "InvalidCharacterError"
        );
        assert!(element.attributes.is_empty());
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
        assert_eq!(element.node.get(), 1);
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
        let mut document = super::Document::from_html_document(&html_document);
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
        let mut document = super::Document::from_html_document(&html_document);

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
        let document = super::Document::from_html_document(&html_document);

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
            document.append_child(second_element),
            Err(DomException::HierarchyRequestError)
        );
        assert_eq!(document.source_document, before);
    }

    #[test]
    fn document_append_child_reports_unknown_node_ids() {
        let mut document = super::Document::new();

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
        let document = super::Document::from_html_document(&html_document);

        assert_eq!(
            document.inner_html(),
            "<!DOCTYPE html><main id=\"content\"><p>A &amp; B</p><!-- note --><br></main>"
        );
    }

    #[test]
    fn new_timer_queue_is_empty() {
        let mut queue = super::TimerQueue::new();
        assert_eq!(queue.pending_count(), 0);
        assert_eq!(queue.pop_next_due(), None);
    }
}
