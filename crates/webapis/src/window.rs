//! The script-visible `Window`, `History`, and `Location` objects.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::atomic::{AtomicU32, Ordering},
    sync::{Arc, Mutex, PoisonError},
    time::Instant,
};

use crate::custom_elements::CustomElementRegistry;
use js::{Function, JsError, JsResult, Object, Realm, Value};
use net::Url;
use security::RendererSecurityContext;

use crate::timer::{TimerId, TimerQueue};

#[derive(Clone)]
enum TimerHandler {
    Callback(Function),
    Source(String),
}

#[derive(Clone)]
struct TimerCallback {
    handler: TimerHandler,
    arguments: Vec<Value>,
    repeat: bool,
    nesting_level: u32,
}

struct AnimationFrameCallbacks {
    next_id: u32,
    callbacks: Vec<(u32, Function)>,
    time_origin: Instant,
}

impl AnimationFrameCallbacks {
    fn new() -> Self {
        Self {
            next_id: 0,
            callbacks: Vec::new(),
            time_origin: Instant::now(),
        }
    }

    fn request(&mut self, function: Function) -> u32 {
        self.next_id = self.next_id.wrapping_add(1);
        let id = self.next_id;
        self.callbacks.push((id, function));
        id
    }

    fn cancel(&mut self, id: u32) {
        self.callbacks.retain(|(callback_id, _)| *callback_id != id);
    }

    fn take_for_frame(&mut self, now: Instant) -> (f64, Vec<u32>) {
        let timestamp = now
            .saturating_duration_since(self.time_origin)
            .as_secs_f64()
            * 1000.0;
        let handles = self.callbacks.iter().map(|(id, _)| *id).collect();
        (timestamp, handles)
    }

    fn take_callback(&mut self, id: u32) -> Option<Function> {
        let index = self
            .callbacks
            .iter()
            .position(|(callback_id, _)| *callback_id == id)?;
        Some(self.callbacks.remove(index).1)
    }
}

struct WindowTimers {
    queue: TimerQueue,
    callbacks: HashMap<TimerId, TimerCallback>,
}

struct MicrotaskQueue {
    callbacks: VecDeque<Function>,
}

impl MicrotaskQueue {
    fn new() -> Self {
        Self {
            callbacks: VecDeque::new(),
        }
    }

    fn enqueue(&mut self, callback: Function) {
        self.callbacks.push_back(callback);
    }

    fn dequeue(&mut self) -> Option<Function> {
        self.callbacks.pop_front()
    }
}

fn drain_microtasks(queue: &Arc<Mutex<MicrotaskQueue>>, evaluator: &js::ScriptEvaluator) -> usize {
    let mut completed = 0;
    loop {
        let callback = queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .dequeue();
        let Some(callback) = callback else {
            break;
        };
        if let Err(error) = evaluator.call(&callback, &[]) {
            log::warn!("window microtask callback threw: {error}");
        }
        completed += 1;
    }
    completed
}

impl WindowTimers {
    fn new() -> Self {
        Self {
            queue: TimerQueue::new(),
            callbacks: HashMap::new(),
        }
    }

    fn set_interval(
        &mut self,
        function: Function,
        delay_ms: u64,
        arguments: Vec<Value>,
    ) -> TimerId {
        let id = self.queue.schedule_interval(delay_ms);
        self.callbacks.insert(
            id,
            TimerCallback {
                handler: TimerHandler::Callback(function),
                arguments,
                repeat: true,
                nesting_level: 1,
            },
        );
        id
    }

    fn clear_interval(&mut self, id: TimerId) -> bool {
        self.callbacks.remove(&id);
        self.queue.cancel(id)
    }

    fn set_timeout(
        &mut self,
        handler: TimerHandler,
        delay_ms: u64,
        arguments: Vec<Value>,
        nesting_level: u32,
    ) -> TimerId {
        let id = self.queue.schedule(delay_ms);
        self.callbacks.insert(
            id,
            TimerCallback {
                handler,
                arguments,
                repeat: false,
                nesting_level,
            },
        );
        id
    }

    fn take_due(&mut self, now: Instant) -> Option<(TimerId, TimerCallback)> {
        let id = self.queue.pop_due(now)?;
        let callback = self.callbacks.get(&id)?.clone();
        if !callback.repeat {
            self.callbacks.remove(&id);
        }
        Some((id, callback))
    }
}

fn timer_delay(value: Option<&Value>) -> JsResult<u64> {
    let number = match value {
        None | Some(Value::Undefined) | Some(Value::Null) => 0.0,
        Some(Value::Number(value)) => *value,
        Some(Value::Boolean(value)) => f64::from(*value),
        Some(Value::String(value)) => value.trim().parse::<f64>().unwrap_or(f64::NAN),
        Some(_) => return Err(JsError::runtime("timer delay must be a number")),
    };
    if number.is_nan() || number <= 0.0 {
        return Ok(0);
    }
    Ok(number.min(u64::MAX as f64).trunc() as u64)
}

fn timer_id(value: Option<&Value>) -> Option<TimerId> {
    let number = match value? {
        Value::Undefined | Value::Null => 0.0,
        Value::Boolean(value) => f64::from(*value),
        Value::Number(value) => *value,
        Value::String(value) => value.trim().parse::<f64>().unwrap_or(f64::NAN),
        Value::Function(_) | Value::Object(_) | Value::Array(_) => return None,
    };
    if !number.is_finite() || number <= 0.0 {
        return None;
    }
    Some(TimerId(number.trunc().min(u32::MAX as f64) as u32))
}

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

#[derive(Clone, Copy)]
struct MediaState {
    viewport: Option<ViewportSize>,
    device_pixel_ratio: f64,
    display_state: &'static str,
    resizable: bool,
}

fn media_state(viewport: Option<ViewportSize>, device_pixel_ratio: f64) -> MediaState {
    MediaState {
        viewport,
        device_pixel_ratio,
        display_state: "normal",
        resizable: true,
    }
}

fn css_length(value: &str) -> Option<f64> {
    value.trim().strip_suffix("px")?.trim().parse().ok()
}

fn css_resolution(value: &str) -> Option<f64> {
    let value = value.trim();
    if let Some(value) = value.strip_suffix("dppx") {
        return value.trim().parse().ok();
    }
    if let Some(value) = value.strip_suffix("dpi") {
        return value.trim().parse::<f64>().ok().map(|value| value / 96.0);
    }
    if let Some(value) = value.strip_suffix("dpcm") {
        return value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value * 2.54 / 96.0);
    }
    None
}

fn media_feature(feature: &str, state: MediaState) -> bool {
    let Some((name, value)) = feature.split_once(':') else {
        return feature.trim() == "resizable" && state.resizable;
    };
    let name = name.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    let width = state.viewport.map_or(0.0, |viewport| viewport.width as f64);
    let height = state
        .viewport
        .map_or(0.0, |viewport| viewport.height as f64);
    match name.as_str() {
        "width" => css_length(&value).is_some_and(|expected| width == expected),
        "min-width" => css_length(&value).is_some_and(|expected| width >= expected),
        "max-width" => css_length(&value).is_some_and(|expected| width <= expected),
        "height" => css_length(&value).is_some_and(|expected| height == expected),
        "min-height" => css_length(&value).is_some_and(|expected| height >= expected),
        "max-height" => css_length(&value).is_some_and(|expected| height <= expected),
        "resolution" => css_resolution(&value)
            .is_some_and(|expected| (state.device_pixel_ratio - expected).abs() < f64::EPSILON),
        "min-resolution" => {
            css_resolution(&value).is_some_and(|expected| state.device_pixel_ratio >= expected)
        }
        "max-resolution" => {
            css_resolution(&value).is_some_and(|expected| state.device_pixel_ratio <= expected)
        }
        "orientation" => match value.as_str() {
            "portrait" => height >= width,
            "landscape" => width > height,
            _ => false,
        },
        "display-state" => value == state.display_state,
        "resizable" => match value.as_str() {
            "true" => state.resizable,
            "false" => !state.resizable,
            _ => false,
        },
        _ => false,
    }
}

fn media_query_matches(query: &str, state: MediaState) -> bool {
    query.split(',').any(|query| {
        let mut query = query.trim().to_ascii_lowercase();
        let negated = query.strip_prefix("not ").is_some();
        if negated {
            query = query[4..].trim().to_owned();
        }
        if let Some(rest) = query.strip_prefix("only ") {
            query = rest.trim().to_owned();
        }
        let mut parts = query.split(" and ");
        let first = parts.next().unwrap_or_default().trim();
        let (media_type, first_feature) = if first.starts_with('(') {
            ("", Some(first))
        } else {
            (first, None)
        };
        let mut matches = media_type.is_empty() || media_type == "all" || media_type == "screen";
        if media_type == "print" || (!media_type.is_empty() && !matches) {
            matches = false;
        }
        for part in first_feature.into_iter().chain(parts) {
            let feature = part
                .trim()
                .strip_prefix('(')
                .and_then(|value| value.strip_suffix(')'));
            matches &= feature.is_some_and(|feature| media_feature(feature.trim(), state));
        }
        if negated { !matches } else { matches }
    })
}

fn media_query_list(query: &str, state: MediaState) -> Object {
    let media = if query.trim().is_empty() {
        "not all".to_owned()
    } else {
        query.trim().to_owned()
    };
    let list = Object::new();
    list.set_read_only("media", Value::String(media.clone()));
    list.set_read_only(
        "matches",
        Value::Boolean(media_query_matches(&media, state)),
    );
    list.set("__query", Value::String(media));
    list.set("__listeners", Value::Object(Object::new()));
    list.set("onchange", Value::Null);
    list
}

fn dispatch_media_query_change(list: &Object, matches: bool) {
    let media = list.get("media").unwrap_or(Value::String(String::new()));
    let event = Object::new();
    event.set("type", Value::String("change".into()));
    event.set("media", media);
    event.set("matches", Value::Boolean(matches));
    event.set("bubbles", Value::Boolean(false));
    event.set("cancelable", Value::Boolean(false));
    event.set("target", Value::Object(list.clone()));
    event.set("currentTarget", Value::Object(list.clone()));
    let mut callbacks = Vec::new();
    if let Some(Value::Object(listeners)) = list.get("__listeners") {
        for index in 0..listeners.len() {
            if let Some(callback) = listeners.get(&index.to_string()) {
                callbacks.push(callback);
            }
        }
    }
    if let Some(callback) = list.get("onchange") {
        callbacks.push(callback);
    }
    for callback in callbacks {
        let result = match callback {
            Value::Function(callback) => callback.call(&[Value::Object(event.clone())]),
            Value::Object(callback) => match callback.get("handleEvent") {
                Some(Value::Function(handle_event)) => {
                    handle_event.call(&[Value::Object(event.clone())])
                }
                _ => Ok(Value::Undefined),
            },
            _ => Ok(Value::Undefined),
        };
        if let Err(error) = result {
            log::warn!("MediaQueryList change listener threw: {error}");
        }
    }
}

fn update_media_query_lists(window: &Object, state: MediaState) {
    let Some(Value::Object(lists)) = window.get("__mediaQueryLists") else {
        return;
    };
    for index in 0..lists.len() {
        let Some(Value::Object(list)) = lists.get(&index.to_string()) else {
            continue;
        };
        let Some(Value::String(query)) = list.get("__query") else {
            continue;
        };
        let matches = media_query_matches(&query, state);
        let old_matches = matches!(list.get("matches"), Some(Value::Boolean(true)));
        if matches != old_matches {
            list.update_read_only("matches", Value::Boolean(matches));
            dispatch_media_query_change(&list, matches);
        }
    }
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
    let timers = Arc::new(Mutex::new(WindowTimers::new()));
    let microtasks = Arc::new(Mutex::new(MicrotaskQueue::new()));
    let animation_frames = Arc::new(Mutex::new(AnimationFrameCallbacks::new()));
    let evaluator = realm.script_evaluator();
    let timer_this = Value::Object(window.clone());
    let nesting_context = Arc::new(AtomicU32::new(0));

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
    window.set("__storageListeners", Value::Object(Object::new()));
    window.set("__mediaQueryLists", Value::Object(Object::new()));

    let media_window = window.clone();
    window.set_method(
        "matchMedia",
        Arc::new(move |arguments| {
            let query = arguments.first().map_or_else(String::new, Value::to_string);
            let current_viewport = match (
                media_window.get("innerWidth"),
                media_window.get("innerHeight"),
            ) {
                (Some(Value::Number(width)), Some(Value::Number(height))) => Some(ViewportSize {
                    width: width.max(0.0) as u32,
                    height: height.max(0.0) as u32,
                }),
                _ => None,
            };
            let current_ratio = match media_window.get("devicePixelRatio") {
                Some(Value::Number(ratio)) => ratio,
                _ => 1.0,
            };
            let list = media_query_list(&query, media_state(current_viewport, current_ratio));
            let listeners = list
                .get("__listeners")
                .expect("MediaQueryList listeners should be installed");
            list.set_method(
                "addEventListener",
                Arc::new({
                    let list = list.clone();
                    move |arguments| {
                        let [Value::String(event_type), callback, ..] = arguments else {
                            return Ok(Value::Undefined);
                        };
                        if event_type == "change"
                            && matches!(callback, Value::Function(_) | Value::Object(_))
                        {
                            let listeners = list
                                .get("__listeners")
                                .and_then(|value| match value {
                                    Value::Object(value) => Some(value),
                                    _ => None,
                                })
                                .expect("MediaQueryList listeners should be installed");
                            if !(0..listeners.len()).any(|index| {
                                listeners
                                    .get(&index.to_string())
                                    .is_some_and(|value| value == *callback)
                            }) {
                                listeners.set(listeners.len().to_string(), callback.clone());
                            }
                        }
                        Ok(Value::Undefined)
                    }
                }),
            );
            let list_for_remove = list.clone();
            list.set_method(
                "removeEventListener",
                Arc::new(move |arguments| {
                    let [Value::String(event_type), callback, ..] = arguments else {
                        return Ok(Value::Undefined);
                    };
                    if event_type == "change"
                        && let Some(Value::Object(listeners)) = list_for_remove.get("__listeners")
                    {
                        for index in 0..listeners.len() {
                            if listeners
                                .get(&index.to_string())
                                .is_some_and(|value| value == *callback)
                            {
                                listeners.set(index.to_string(), Value::Undefined);
                            }
                        }
                    }
                    Ok(Value::Undefined)
                }),
            );
            let list_for_add = list.clone();
            list.set_method(
                "addListener",
                Arc::new(move |arguments| {
                    if let Some(callback) = arguments.first()
                        && let Some(Value::Function(add)) = list_for_add.get("addEventListener")
                    {
                        add.call(&[Value::String("change".into()), callback.clone()])?;
                    }
                    Ok(Value::Undefined)
                }),
            );
            let list_for_remove_listener = list.clone();
            list.set_method(
                "removeListener",
                Arc::new(move |arguments| {
                    if let Some(callback) = arguments.first()
                        && let Some(Value::Function(remove)) =
                            list_for_remove_listener.get("removeEventListener")
                    {
                        remove.call(&[Value::String("change".into()), callback.clone()])?;
                    }
                    Ok(Value::Undefined)
                }),
            );
            if let Value::Object(lists) =
                media_window.get("__mediaQueryLists").expect("media lists")
            {
                lists.set(lists.len().to_string(), Value::Object(list.clone()));
            }
            drop(listeners);
            Ok(Value::Object(list))
        }),
    );
    let update_media_window = window.clone();
    window.set_method(
        "__updateMediaQueries",
        Arc::new(move |arguments| {
            let width = arguments.first().and_then(|value| match value {
                Value::Number(value) => Some(*value as u32),
                _ => None,
            });
            let height = arguments.get(1).and_then(|value| match value {
                Value::Number(value) => Some(*value as u32),
                _ => None,
            });
            let ratio = arguments
                .get(2)
                .and_then(|value| match value {
                    Value::Number(value) => Some(*value),
                    _ => None,
                })
                .unwrap_or(1.0);
            update_media_query_lists(
                &update_media_window,
                media_state(
                    width
                        .zip(height)
                        .map(|(width, height)| ViewportSize { width, height }),
                    ratio,
                ),
            );
            Ok(Value::Undefined)
        }),
    );

    let timers_for_timeout = Arc::clone(&timers);
    let nesting_for_timeout = Arc::clone(&nesting_context);
    window.set_method(
        "setTimeout",
        Arc::new(move |arguments| {
            let handler = match arguments.first() {
                Some(Value::Function(function)) => TimerHandler::Callback(function.clone()),
                Some(value) => TimerHandler::Source(value.to_string()),
                None => return Err(JsError::runtime("setTimeout() requires a handler")),
            };
            let nesting = nesting_for_timeout.load(Ordering::Acquire);
            let requested_delay = timer_delay(arguments.get(1))?;
            let delay = if nesting > 5 && requested_delay < 4 {
                4
            } else {
                requested_delay
            };
            let callback_arguments = arguments.get(2..).unwrap_or_default().to_vec();
            let id = timers_for_timeout
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .set_timeout(handler, delay, callback_arguments, nesting + 1);
            Ok(Value::Number(f64::from(id.0)))
        }),
    );
    if let Some(timeout) = window.get("setTimeout") {
        realm.register_global_value("setTimeout", timeout)?;
    }

    let microtasks_for_queue = Arc::clone(&microtasks);
    window.set_method(
        "queueMicrotask",
        Arc::new(move |arguments| {
            let Some(Value::Function(function)) = arguments.first() else {
                return Err(JsError::runtime(
                    "queueMicrotask() requires a function callback",
                ));
            };
            microtasks_for_queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .enqueue(function.clone());
            Ok(Value::Undefined)
        }),
    );

    let timers_for_interval = Arc::clone(&timers);
    window.set_method(
        "setInterval",
        Arc::new(move |arguments| {
            let Some(Value::Function(function)) = arguments.first() else {
                return Err(JsError::runtime(
                    "setInterval() requires a function callback",
                ));
            };
            let delay = timer_delay(arguments.get(1))?;
            let callback_arguments = arguments.iter().skip(2).cloned().collect();
            let mut timers = timers_for_interval
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            Ok(Value::Number(
                timers
                    .set_interval(function.clone(), delay, callback_arguments)
                    .0 as f64,
            ))
        }),
    );

    let timers_for_clear = Arc::clone(&timers);
    let clear_interval: js::HostFunction = Arc::new(move |arguments: &[Value]| {
        if let Some(id) = timer_id(arguments.first()) {
            timers_for_clear
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear_interval(id);
        }
        Ok(Value::Undefined)
    });
    window.set_method("clearInterval", Arc::clone(&clear_interval));
    window.set_method("clearTimeout", clear_interval);

    let animation_frames_for_request = Arc::clone(&animation_frames);
    window.set_method(
        "requestAnimationFrame",
        Arc::new(move |arguments| {
            let Some(Value::Function(function)) = arguments.first() else {
                return Err(JsError::runtime(
                    "requestAnimationFrame() requires a function callback",
                ));
            };
            let id = animation_frames_for_request
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .request(function.clone());
            Ok(Value::Number(id as f64))
        }),
    );

    let animation_frames_for_cancel = Arc::clone(&animation_frames);
    window.set_method(
        "cancelAnimationFrame",
        Arc::new(move |arguments| {
            if let Some(Value::Number(id)) = arguments.first()
                && id.is_finite()
                && *id >= 0.0
            {
                animation_frames_for_cancel
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .cancel(id.trunc().min(u32::MAX as f64) as u32);
            }
            Ok(Value::Undefined)
        }),
    );

    let animation_frames_for_pump = Arc::clone(&animation_frames);
    let microtasks_for_animation = Arc::clone(&microtasks);
    let evaluator_for_animation = evaluator.clone();
    window.set_method(
        "__runAnimationFrames",
        Arc::new(move |_| {
            let (timestamp, handles) = animation_frames_for_pump
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take_for_frame(Instant::now());
            let mut fired = 0;
            for handle in handles {
                let callback = animation_frames_for_pump
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take_callback(handle);
                let Some(callback) = callback else {
                    continue;
                };
                if let Err(error) = callback.call(&[Value::Number(timestamp)]) {
                    log::warn!("requestAnimationFrame callback threw: {error}");
                }
                fired += 1;
                drain_microtasks(&microtasks_for_animation, &evaluator_for_animation);
            }
            Ok(Value::Number(fired as f64))
        }),
    );

    let timers_for_pump = Arc::clone(&timers);
    let microtasks_for_timer = Arc::clone(&microtasks);
    let evaluator_for_timers = evaluator.clone();
    window.set_method(
        "__runTimers",
        Arc::new(move |_| {
            let mut fired = 0;
            let mut delivered = HashSet::new();
            loop {
                let callback = timers_for_pump
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take_due(Instant::now());
                let Some((id, callback)) = callback else {
                    break;
                };
                if !delivered.insert(id) {
                    break;
                }
                let previous_nesting =
                    nesting_context.swap(callback.nesting_level, Ordering::AcqRel);
                let result = match callback.handler {
                    TimerHandler::Callback(function) => evaluator_for_timers.call_with_this(
                        &function,
                        timer_this.clone(),
                        &callback.arguments,
                    ),
                    TimerHandler::Source(source) => {
                        evaluator_for_timers.evaluate_script_with_this(&source, timer_this.clone())
                    }
                };
                nesting_context.store(previous_nesting, Ordering::Release);
                if let Err(error) = result {
                    log::warn!("window timer callback threw: {error}");
                }
                fired += 1;
                drain_microtasks(&microtasks_for_timer, &evaluator_for_timers);
            }
            Ok(Value::Number(fired as f64))
        }),
    );

    let microtasks_for_pump = Arc::clone(&microtasks);
    let evaluator_for_microtasks = evaluator.clone();
    window.set_method(
        "__runMicrotasks",
        Arc::new(move |_| {
            let completed = drain_microtasks(&microtasks_for_pump, &evaluator_for_microtasks);
            Ok(Value::Number(completed as f64))
        }),
    );

    let listeners_window = window.clone();
    window.set_method(
        "addEventListener",
        Arc::new(move |arguments| {
            let [Value::String(event_type), Value::Function(callback), ..] = arguments else {
                return Err(JsError::runtime(
                    "window.addEventListener() expects an event type and callback",
                ));
            };
            if event_type == "popstate" || event_type == "storage" {
                let list_name = if event_type == "popstate" {
                    "__popstateListeners"
                } else {
                    "__storageListeners"
                };
                let list = match listeners_window.get(list_name) {
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
            if (event_type == "popstate" || event_type == "storage")
                && let Some(Value::Object(list)) = remove_window.get(if event_type == "popstate" {
                    "__popstateListeners"
                } else {
                    "__storageListeners"
                })
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
    realm.register_global_value(
        "matchMedia",
        window
            .get("matchMedia")
            .expect("matchMedia should be installed on Window"),
    )?;
    realm.register_global_value(
        "setInterval",
        window
            .get("setInterval")
            .expect("setInterval should be installed on Window"),
    )?;
    realm.register_global_value(
        "queueMicrotask",
        window
            .get("queueMicrotask")
            .expect("queueMicrotask should be installed on Window"),
    )?;
    realm.register_global_value(
        "clearInterval",
        window
            .get("clearInterval")
            .expect("clearInterval should be installed on Window"),
    )?;
    realm.register_global_value(
        "clearTimeout",
        window
            .get("clearTimeout")
            .expect("clearTimeout should be installed on Window"),
    )?;
    realm.register_global_value(
        "requestAnimationFrame",
        window
            .get("requestAnimationFrame")
            .expect("requestAnimationFrame should be installed on Window"),
    )?;
    realm.register_global_value(
        "cancelAnimationFrame",
        window
            .get("cancelAnimationFrame")
            .expect("cancelAnimationFrame should be installed on Window"),
    )?;
    Ok(window)
}

/// Runs callbacks whose `setInterval()` deadlines have elapsed.
///
/// Browser integrations call this from the owning page/event-loop thread.
/// The timer API queues work and never invokes JavaScript from a background
/// thread. The return value is the number of callbacks delivered.
/// String handlers are intentionally unsupported because dynamic script
/// compilation is not yet exposed by this engine.
pub fn run_window_timers(window: &Object) -> JsResult<usize> {
    let Some(Value::Function(pump)) = window.get("__runTimers") else {
        return Err(JsError::runtime("window timer pump is not installed"));
    };
    match pump.call(&[]) {
        Ok(Value::Number(fired)) => Ok(fired as usize),
        Ok(_) => Err(JsError::runtime("window timer pump returned a non-number")),
        Err(error) => Err(error),
    }
}

/// Runs the Window's microtask checkpoint.
///
/// Browser integrations call this from the owning page/event-loop thread
/// after a JavaScript task or other callback completes. Microtasks queued by a
/// microtask are also run during the same checkpoint. A callback exception is
/// reported and does not prevent later microtasks from running.
pub fn run_window_microtasks(window: &Object) -> JsResult<usize> {
    let Some(Value::Function(pump)) = window.get("__runMicrotasks") else {
        return Err(JsError::runtime("window microtask pump is not installed"));
    };
    match pump.call(&[]) {
        Ok(Value::Number(completed)) => Ok(completed as usize),
        Ok(_) => Err(JsError::runtime(
            "window microtask pump returned a non-number",
        )),
        Err(error) => Err(error),
    }
}

/// Runs the pending `window.requestAnimationFrame()` callbacks for one frame.
///
/// Browser integrations call this from the owning page/event-loop thread
/// immediately before presenting a frame. The callbacks receive the same
/// monotonic timestamp for the frame and callbacks registered during a frame
/// are deferred until the next frame.
pub fn run_window_animation_frames(window: &Object) -> JsResult<usize> {
    let Some(Value::Function(pump)) = window.get("__runAnimationFrames") else {
        return Err(JsError::runtime(
            "window animation frame pump is not installed",
        ));
    };
    match pump.call(&[]) {
        Ok(Value::Number(fired)) => Ok(fired as usize),
        Ok(_) => Err(JsError::runtime(
            "window animation frame pump returned a non-number",
        )),
        Err(error) => Err(error),
    }
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
