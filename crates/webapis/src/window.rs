//! The script-visible `Window`, `History`, and `Location` objects.

use std::sync::{Arc, Mutex, PoisonError};

use crate::custom_elements::CustomElementRegistry;
use js::{JsError, JsResult, Object, Realm, Value};
use net::Url;
use security::RendererSecurityContext;

#[derive(Clone, Debug)]
struct HistoryEntry {
    url: Url,
    state: Value,
}

#[derive(Debug)]
struct SessionHistory {
    entries: Vec<HistoryEntry>,
    current: usize,
}

impl SessionHistory {
    fn new(url: &str) -> JsResult<Self> {
        Ok(Self {
            entries: vec![HistoryEntry {
                url: Url::parse(url).map_err(|error| JsError::runtime(error.to_string()))?,
                state: Value::Null,
            }],
            current: 0,
        })
    }

    fn current(&self) -> &HistoryEntry {
        &self.entries[self.current]
    }
}

fn lock_history(history: &Mutex<SessionHistory>) -> std::sync::MutexGuard<'_, SessionHistory> {
    history.lock().unwrap_or_else(PoisonError::into_inner)
}

fn clone_for_storage(value: &Value) -> JsResult<Value> {
    match value {
        Value::Undefined
        | Value::Null
        | Value::Boolean(_)
        | Value::Number(_)
        | Value::String(_) => Ok(value.clone()),
        Value::Function(_) => Err(JsError::runtime("history state cannot contain a function")),
        Value::Object(object) | Value::Array(object) => {
            let cloned = Object::new();
            for key in object.keys() {
                if let Some(property) = object.get_own(&key) {
                    cloned.set(key, clone_for_storage(&property)?);
                }
            }
            Ok(if matches!(value, Value::Array(_)) {
                Value::Array(cloned)
            } else {
                Value::Object(cloned)
            })
        }
    }
}

fn resolve_history_url(base: &Url, requested: Option<&Value>) -> JsResult<Url> {
    let Some(requested) = requested else {
        return Ok(base.clone());
    };
    let Value::String(requested) = requested else {
        return Err(JsError::runtime("history URL must be a string or null"));
    };
    if requested.is_empty() {
        return Ok(base.clone());
    }
    let resolved = base
        .join(requested)
        .map_err(|error| JsError::runtime(format!("invalid history URL: {error}")))?;
    if resolved.origin() != base.origin() {
        return Err(JsError::runtime(
            "history URL must have the same origin as the document",
        ));
    }
    Ok(resolved)
}

fn sync_history_properties(history: &Object, location: &Object, session: &SessionHistory) {
    history.set("length", Value::Number(session.entries.len() as f64));
    history.set("state", session.current().state.clone());
    location.set(
        "href",
        Value::String(session.current().url.as_str().to_owned()),
    );
}

fn invoke_popstate(window: &Object, state: &Value) {
    let event = Object::new();
    event.set("type", Value::String("popstate".into()));
    event.set("state", state.clone());
    event.set("bubbles", Value::Boolean(false));
    event.set("cancelable", Value::Boolean(false));
    event.set("target", Value::Object(window.clone()));
    event.set("currentTarget", Value::Object(window.clone()));

    let mut callbacks = Vec::new();
    if let Some(Value::Object(list)) = window.get("__popstateListeners") {
        for index in 0..list.len() {
            if let Some(callback) = list.get(&index.to_string()) {
                callbacks.push(callback);
            }
        }
    }
    if let Some(Value::Function(callback)) = window.get("onpopstate") {
        callbacks.push(Value::Function(callback));
    }
    for callback in callbacks {
        if let Value::Function(callback) = callback
            && let Err(error) = callback.call(&[Value::Object(event.clone())])
        {
            log::warn!("popstate listener threw: {error}");
        }
    }
}

/// The viewport dimensions exposed to a script-visible Window.
///
/// Dimensions are CSS pixels, matching CSSOM View. A viewport is optional
/// because a Window without an associated viewport must report zero for
/// `innerWidth` and `innerHeight`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSize {
    /// The viewport width in CSS pixels.
    pub width: u32,
    /// The viewport height in CSS pixels.
    pub height: u32,
}

fn register_window_object(
    realm: &mut Realm,
    initial_url: &str,
    device_pixel_ratio: f64,
    viewport: Option<ViewportSize>,
) -> JsResult<Object> {
    let window = match realm.registered_global_value("window") {
        Some(Value::Object(window)) => window,
        _ => Object::new(),
    };
    let self_window = window.clone();
    let navigator = Object::new();
    let navigator_for_window = navigator.clone();
    let history = Object::new();
    let location = Object::new();
    let session = Arc::new(Mutex::new(SessionHistory::new(initial_url)?));
    let custom_elements = match realm.registered_global_value("customElements") {
        Some(Value::Object(registry)) => registry,
        _ => CustomElementRegistry::new().object(),
    };

    window.set_method(
        "self",
        Arc::new(move |_| Ok(Value::Object(self_window.clone()))),
    );
    window.set_method(
        "navigator",
        Arc::new(move |_| Ok(Value::Object(navigator_for_window.clone()))),
    );
    window.set("customElements", Value::Object(custom_elements.clone()));
    // CSSOM View exposes scrollX as a readonly number. The renderer does not
    // yet have document horizontal scrolling, so the initial offset is zero.
    window.set_read_only("scrollX", Value::Number(0.0));
    // CSSOM View exposes scrollY and pageYOffset as readonly numbers. The
    // browser shell updates these host properties as the viewport scrolls.
    window.set_read_only("scrollY", Value::Number(0.0));
    window.set_read_only("pageYOffset", Value::Number(0.0));
    // The desktop shell supplies the output device scale; non-windowed
    // callers use the specification's fallback value of 1.0.
    window.set_replaceable_read_only("devicePixelRatio", Value::Number(device_pixel_ratio));
    // CSSOM View: innerWidth is the viewport width in CSS pixels, or zero
    // when the Window has no associated viewport. It is replaceable.
    let inner_width = Value::Number(viewport.map_or(0, |size| size.width) as f64);
    window.set("innerWidth", inner_width.clone());
    // CSSOM View: innerHeight is the viewport height in CSS pixels, including
    // any rendered scrollbar, or zero when the Window has no viewport. It is
    // replaceable.
    let inner_height = Value::Number(viewport.map_or(0, |size| size.height) as f64);
    window.set("innerHeight", inner_height.clone());
    window.set("onpopstate", Value::Null);
    history.set("scrollRestoration", Value::String("auto".into()));
    window.set("__popstateListeners", Value::Object(Object::new()));

    let listeners_window = window.clone();
    window.set_method(
        "addEventListener",
        Arc::new(move |arguments| {
            let [Value::String(event_type), Value::Function(callback), ..] = arguments else {
                return Err(JsError::runtime(
                    "window.addEventListener() expects an event type and callback",
                ));
            };
            if event_type == "popstate" {
                let list = match listeners_window.get("__popstateListeners") {
                    Some(Value::Object(list)) => list,
                    _ => return Ok(Value::Undefined),
                };
                let index = list.len();
                list.set(index.to_string(), Value::Function(callback.clone()));
            }
            Ok(Value::Undefined)
        }),
    );

    let remove_window = window.clone();
    window.set_method(
        "removeEventListener",
        Arc::new(move |arguments| {
            let [Value::String(event_type), Value::Function(callback), ..] = arguments else {
                return Ok(Value::Undefined);
            };
            if event_type == "popstate"
                && let Some(Value::Object(list)) = remove_window.get("__popstateListeners")
            {
                for index in 0..list.len() {
                    if list
                        .get(&index.to_string())
                        .is_some_and(|value| value == Value::Function(callback.clone()))
                    {
                        list.set(index.to_string(), Value::Undefined);
                    }
                }
            }
            Ok(Value::Undefined)
        }),
    );

    let history_for_window = history.clone();
    window.set_method(
        "history",
        Arc::new(move |_| Ok(Value::Object(history_for_window.clone()))),
    );
    let location_for_window = location.clone();
    window.set_method(
        "location",
        Arc::new(move |_| Ok(Value::Object(location_for_window.clone()))),
    );

    let initial_session = lock_history(&session);
    sync_history_properties(&history, &location, &initial_session);
    drop(initial_session);

    let history_for_back = history.clone();
    let location_for_back = location.clone();
    let window_for_back = window.clone();
    let session_for_back = Arc::clone(&session);
    history.set_method(
        "back",
        Arc::new(move |_| {
            let mut session = lock_history(&session_for_back);
            if session.current > 0 {
                session.current -= 1;
                sync_history_properties(&history_for_back, &location_for_back, &session);
                invoke_popstate(&window_for_back, &session.current().state);
            }
            Ok(Value::Undefined)
        }),
    );

    let history_for_forward = history.clone();
    let location_for_forward = location.clone();
    let window_for_forward = window.clone();
    let session_for_forward = Arc::clone(&session);
    history.set_method(
        "forward",
        Arc::new(move |_| {
            let mut session = lock_history(&session_for_forward);
            if session.current + 1 < session.entries.len() {
                session.current += 1;
                sync_history_properties(&history_for_forward, &location_for_forward, &session);
                invoke_popstate(&window_for_forward, &session.current().state);
            }
            Ok(Value::Undefined)
        }),
    );

    let history_for_go = history.clone();
    let location_for_go = location.clone();
    let window_for_go = window.clone();
    let session_for_go = Arc::clone(&session);
    history.set_method(
        "go",
        Arc::new(move |arguments| {
            let delta = match arguments.first() {
                None | Some(Value::Undefined) => 0,
                Some(Value::Number(value)) if value.is_finite() => value.trunc() as i64,
                _ => return Err(JsError::runtime("history.go() requires a finite number")),
            };
            let mut session = lock_history(&session_for_go);
            let target = (session.current as i64 + delta)
                .clamp(0, session.entries.len().saturating_sub(1) as i64)
                as usize;
            if target != session.current {
                session.current = target;
                sync_history_properties(&history_for_go, &location_for_go, &session);
                invoke_popstate(&window_for_go, &session.current().state);
            }
            Ok(Value::Undefined)
        }),
    );

    let history_for_push = history.clone();
    let location_for_push = location.clone();
    let session_for_push = Arc::clone(&session);
    history.set_method(
        "pushState",
        Arc::new(move |arguments| {
            let [state, _unused, url @ ..] = arguments else {
                return Err(JsError::runtime(
                    "history.pushState() expects data and an unused title",
                ));
            };
            let state = clone_for_storage(state)?;
            let mut session = lock_history(&session_for_push);
            let new_url = resolve_history_url(&session.current().url, url.first())?;
            let current = session.current;
            session.entries.truncate(current + 1);
            session.entries.push(HistoryEntry {
                url: new_url,
                state,
            });
            session.current += 1;
            sync_history_properties(&history_for_push, &location_for_push, &session);
            Ok(Value::Undefined)
        }),
    );

    let history_for_replace = history.clone();
    let location_for_replace = location.clone();
    let session_for_replace = Arc::clone(&session);
    history.set_method(
        "replaceState",
        Arc::new(move |arguments| {
            let [state, _unused, url @ ..] = arguments else {
                return Err(JsError::runtime(
                    "history.replaceState() expects data and an unused title",
                ));
            };
            let state = clone_for_storage(state)?;
            let mut session = lock_history(&session_for_replace);
            let new_url = resolve_history_url(&session.current().url, url.first())?;
            let current = session.current;
            session.entries[current] = HistoryEntry {
                url: new_url,
                state,
            };
            sync_history_properties(&history_for_replace, &location_for_replace, &session);
            Ok(Value::Undefined)
        }),
    );

    realm.register_global_value("window", Value::Object(window.clone()))?;
    realm.register_global_value("self", Value::Object(window.clone()))?;
    realm.register_global_value("customElements", Value::Object(custom_elements))?;
    realm.register_global_value("navigator", Value::Object(navigator))?;
    realm.register_global_value("innerWidth", inner_width)?;
    realm.register_global_value("innerHeight", inner_height)?;
    Ok(window)
}

/// Registers the script-visible window globals using `about:blank` as its URL.
pub fn register_window(realm: &mut Realm) -> JsResult<()> {
    register_window_object(realm, "about:blank", 1.0, None).map(|_| ())
}

/// Registers Window globals with the immutable renderer security context's
/// serialized origin exposed as `window.origin` and the global `origin`.
///
/// The origin property follows the current browser API subset and is
/// replaceable by scripts; the security context itself remains owned by the
/// renderer and is not exposed for mutation.
pub fn register_window_with_security_context(
    realm: &mut Realm,
    security_context: RendererSecurityContext,
) -> JsResult<()> {
    let window = register_window_object(realm, "about:blank", 1.0, None)?;
    let serialized_origin = security_context.origin().ascii_serialization();
    window.set("origin", Value::String(serialized_origin.clone()));
    realm.register_global_value("origin", Value::String(serialized_origin))?;
    Ok(())
}

/// Registers the script-visible window globals and returns the shared window
/// object so document bindings can add `document` without replacing it.
///
/// History traversal is implemented for same-document entries. The native
/// browser shell still owns cross-document navigation, so traversing an entry
/// created by a separate loaded document is not routed back to that shell.
pub fn register_window_with_url(realm: &mut Realm, url: &str) -> JsResult<Object> {
    register_window_object(realm, url, 1.0, None)
}

/// Registers the script-visible window globals for `url` with the output
/// device scale exposed as the read-only `window.devicePixelRatio` value.
///
/// `device_pixel_ratio` is the number of device pixels per CSS pixel and must
/// be finite and greater than zero. Call this before evaluating page scripts.
pub fn register_window_with_url_and_device_pixel_ratio(
    realm: &mut Realm,
    url: &str,
    device_pixel_ratio: f64,
) -> JsResult<Object> {
    register_window_with_url_device_pixel_ratio_and_viewport(realm, url, device_pixel_ratio, None)
}

/// Registers Window globals with a renderer security context and viewport.
///
/// `viewport` is in CSS pixels; its dimensions are exposed as
/// `window.innerWidth`, `window.innerHeight`, and their global aliases.
pub fn register_window_with_security_context_and_viewport(
    realm: &mut Realm,
    security_context: RendererSecurityContext,
    viewport: ViewportSize,
) -> JsResult<()> {
    register_window_with_security_context(realm, security_context)?;
    register_window_object(realm, "about:blank", 1.0, Some(viewport)).map(|_| ())
}

/// Registers the script-visible window globals for `url` with the output
/// device scale and, optionally, the viewport exposed to scripts.
///
/// `device_pixel_ratio` is the number of device pixels per CSS pixel and must
/// be finite and greater than zero. `viewport` is in CSS pixels; without one,
/// `window.innerWidth` and `window.innerHeight` are zero. Call this before
/// evaluating page scripts.
pub fn register_window_with_url_device_pixel_ratio_and_viewport(
    realm: &mut Realm,
    url: &str,
    device_pixel_ratio: f64,
    viewport: Option<ViewportSize>,
) -> JsResult<Object> {
    if !device_pixel_ratio.is_finite() || device_pixel_ratio <= 0.0 {
        return Err(JsError::runtime(
            "window.devicePixelRatio must be finite and greater than zero",
        ));
    }
    register_window_object(realm, url, device_pixel_ratio, viewport)
}
