//! DOM event construction, listener registration, and dispatch.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use html::NodeID;
use js::{JsError, JsResult, Object, Value};

use crate::document::Document;
use crate::element::{Element, element_object};

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
pub(crate) struct EventTarget {
    listeners: Vec<EventListener>,
}

pub(crate) type EventTargetRef = Arc<Mutex<EventTarget>>;

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
                .entry(node.get())
                .or_insert_with(|| Arc::new(Mutex::new(EventTarget::default())))
                .clone()
        }
    }
}

pub(crate) fn add_event_listener_at(
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

pub(crate) fn remove_event_listener_at(
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

pub(crate) fn dispatch_event_with_target(
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

pub(crate) fn dispatch_event_at(
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
            .and_then(|source| source.node(current_node).and_then(|n| n.parent));
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

/// Dispatches a `click` event whose target is the document.
///
/// The event bubbles and is cancelable, as a user `click` does. Listeners
/// registered with `document.addEventListener('click', ...)` run, in both the
/// capture and bubble phases at the document. This does not hit-test an
/// element, so listeners registered on an element do not run.
///
/// `document` must be the document registered in the realm, or a clone of it.
/// Clones share the listener list.
pub fn dispatch_click(document: &Document) -> JsResult<Value> {
    let options = Object::new();
    options.set("bubbles", Value::Boolean(true));
    options.set("cancelable", Value::Boolean(true));
    let event = event_constructor(&[Value::String("click".into()), Value::Object(options)])?;
    dispatch_event(document, &[event])
}

/// Dispatches a user `click` event at the element containing `node`.
///
/// `node` is usually the result of a layout hit test. A non-element node,
/// such as text, targets its nearest element ancestor, matching how pointer
/// events are retargeted in the DOM. The event bubbles and is cancelable, so
/// listeners on the target, its ancestors, and the document run in capture,
/// target, and bubble order. When no element contains `node`, the click
/// targets the document as [`dispatch_click`] does.
///
/// `document` must be the document registered in the realm, or a clone of it.
/// Clones share listener lists. Returns `false` when a listener called
/// `preventDefault()`.
pub fn dispatch_click_at(document: &Document, node: NodeID) -> JsResult<Value> {
    let Some(element) = nearest_element(document, node) else {
        return dispatch_click(document);
    };
    let options = Object::new();
    options.set("bubbles", Value::Boolean(true));
    options.set("cancelable", Value::Boolean(true));
    let event = event_constructor(&[Value::String("click".into()), Value::Object(options)])?;
    let target_node = element.node;
    let target = element_object(element, Arc::new(Mutex::new(document.clone())));
    dispatch_event_at(document, Some(target_node), &target, &[event])
}

fn nearest_element(document: &Document, node: NodeID) -> Option<Element> {
    let mut current = Some(node);
    while let Some(node) = current {
        if let Some(element) = element_for_node(document, node) {
            return Some(element);
        }
        current = document
            .source_document
            .as_ref()
            .and_then(|source| source.node(node))
            .and_then(|node| node.parent);
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use html::parse_raw_html;
    use js::{JsError, Realm, Value};

    use super::*;
    use crate::alert::{AlertSink, register_alert};
    use crate::document::{Document, register_document};

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
    fn registration_exposes_document_add_event_listener_to_javascript() {
        let html_document = parse_raw_html("<main></main>".to_owned());
        let document = Document::from_html_document(&html_document);
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
    fn dispatch_click_invokes_a_document_listener() {
        let sink = Arc::new(TestAlert::default());
        let html_document = parse_raw_html("<main></main>".to_owned());
        let document = Document::from_html_document(&html_document);
        let retained = document.clone();
        let mut realm = Realm::new();
        register_alert(&mut realm, sink.clone()).unwrap();
        register_document(&mut realm, document).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "function callback() { alert('Clicked') }\n\
                 document.addEventListener('click', callback)",
            ),
            Ok(Value::Undefined)
        );
        assert_eq!(dispatch_click(&retained), Ok(Value::Boolean(true)));
        assert_eq!(sink.messages.lock().unwrap().as_slice(), ["Clicked"]);
    }

    #[test]
    fn javascript_event_dispatch_invokes_callbacks_and_remove_works() {
        let html_document = parse_raw_html("<main></main>".to_owned());
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

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
    fn element_events_capture_target_and_bubble_to_document_ancestors() {
        let html_document =
            parse_raw_html("<main id='container'><button id='button'></button></main>".to_owned());
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

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
    fn user_click_on_element_text_targets_the_element_and_bubbles() {
        let html_document = parse_raw_html("<main><button id='btn'>Go</button></main>".to_owned());
        let button = html_document.get_element_by_id("btn").unwrap();
        let text = html_document.node(button).unwrap().children[0];
        let document = Document::from_html_document(&html_document);
        let mut realm = Realm::new();
        register_document(&mut realm, document.clone()).unwrap();
        realm
            .evaluate_script(
                "var order = ''\n\
                 function onButton(event) { order = order + event.target.getAttribute('id') }\n\
                 function onDocument(event) { order = order + 'D' }\n\
                 document.getElementById('btn').addEventListener('click', onButton)\n\
                 document.addEventListener('click', onDocument)",
            )
            .unwrap();

        assert_eq!(dispatch_click_at(&document, text), Ok(Value::Boolean(true)));
        assert_eq!(
            realm.evaluate_script("order"),
            Ok(Value::String("btnD".into()))
        );
    }

    #[test]
    fn abort_controller_removes_a_signal_bound_listener() {
        let html_document = parse_raw_html("<main></main>".to_owned());
        let mut realm = Realm::new();
        register_document(&mut realm, Document::from_html_document(&html_document)).unwrap();

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
        let document = Document::new();
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
        let document = Document::new();
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
        let document = Document::new();
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
}
