//! The script-visible `Window`, `History`, and `Location` objects.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::custom_elements::CustomElementRegistry;
use js::{Function, JsError, JsResult, Object, Realm, Value};
use net::Url;
use security::RendererSecurityContext;

enum TimerHandler {
    Callback(Function),
    Source(String),
}

struct PendingTimer {
    cancelled: Arc<AtomicBool>,
    handler: TimerHandler,
    arguments: Vec<Value>,
    nesting_level: u32,
    nesting_context: Arc<AtomicU32>,
    evaluator: js::ScriptEvaluator,
    receiver: Value,
}

struct TimerJob {
    deadline: Instant,
    id: u32,
    cancelled: Arc<AtomicBool>,
    dispatcher: TimerDispatcher,
}

/// Callback used by a host to enqueue a timer on its page task loop.
pub type TimerDispatcher = Arc<dyn Fn(u32) + Send + Sync>;

fn fire_pending_timer(timers: &Mutex<HashMap<u32, PendingTimer>>, id: u32) {
    let timer = timers
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
    let Some(timer) = timer else { return };
    if timer.cancelled.load(Ordering::Acquire) {
        return;
    }
    let previous_level = timer
        .nesting_context
        .swap(timer.nesting_level, Ordering::AcqRel);
    let result = match timer.handler {
        TimerHandler::Callback(callback) => {
            timer
                .evaluator
                .call_with_this(&callback, timer.receiver, &timer.arguments)
        }
        TimerHandler::Source(source) => timer
            .evaluator
            .evaluate_script_with_this(&source, timer.receiver),
    };
    timer
        .nesting_context
        .store(previous_level, Ordering::Release);
    if let Err(error) = result {
        log::warn!("setTimeout callback threw: {error}");
    }
}

fn timer_scheduler() -> &'static mpsc::Sender<TimerJob> {
    static SCHEDULER: OnceLock<mpsc::Sender<TimerJob>> = OnceLock::new();
    SCHEDULER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<TimerJob>();
        thread::spawn(move || {
            let mut jobs = Vec::new();
            loop {
                if jobs.is_empty() {
                    match receiver.recv() {
                        Ok(job) => jobs.push(job),
                        Err(_) => break,
                    }
                    continue;
                }
                let next = jobs
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, job)| job.deadline)
                    .map(|(index, job)| (index, job.deadline));
                let Some((index, deadline)) = next else {
                    continue;
                };
                match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(job) => jobs.push(job),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        let job = jobs.remove(index);
                        if !job.cancelled.load(Ordering::Acquire) {
                            (job.dispatcher)(job.id);
                        }
                    }
                }
            }
        });
        sender
    })
}

fn webidl_long(value: &Value) -> JsResult<i32> {
    let number = match value {
        Value::Undefined => 0.0,
        Value::Null => 0.0,
        Value::Boolean(value) => f64::from(u8::from(*value)),
        Value::Number(value) => *value,
        Value::String(value) => {
            if value.trim().is_empty() {
                0.0
            } else {
                value.trim().parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Value::Array(_) | Value::Object(_) => {
            value.to_string().trim().parse::<f64>().unwrap_or(f64::NAN)
        }
        Value::Function(_) => f64::NAN,
    };
    if !number.is_finite() || number == 0.0 {
        return Ok(0);
    }
    let integer = number.trunc();
    let wrapped = integer.rem_euclid(4_294_967_296.0);
    Ok(if wrapped >= 2_147_483_648.0 {
        (wrapped - 4_294_967_296.0) as i32
    } else {
        wrapped as i32
    })
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
        if let Value::Function(callback) = callback {
            if let Err(error) = callback.call(&[Value::Object(event.clone())]) {
                log::warn!("popstate listener threw: {error}");
            }
        }
    }
}

fn register_window_object(
    realm: &mut Realm,
    initial_url: &str,
    device_pixel_ratio: f64,
    timer_dispatcher: Option<TimerDispatcher>,
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
    // The desktop shell supplies the output device scale; non-windowed
    // callers use the specification's fallback value of 1.0.
    window.set_replaceable_read_only("devicePixelRatio", Value::Number(device_pixel_ratio));
    window.set("onpopstate", Value::Null);
    let timers: Arc<Mutex<HashMap<u32, PendingTimer>>> = Arc::new(Mutex::new(HashMap::new()));
    let timers_for_set = Arc::clone(&timers);
    let next_timer_id = Arc::new(AtomicU32::new(0));
    let next_timer_id_for_set = Arc::clone(&next_timer_id);
    let nesting_context = Arc::new(AtomicU32::new(0));
    let nesting_context_for_set = Arc::clone(&nesting_context);
    let evaluator = realm.script_evaluator();
    let timer_receiver = Value::Object(window.clone());
    let timer_dispatcher = timer_dispatcher.unwrap_or_else(|| {
        let timers = Arc::clone(&timers);
        Arc::new(move |id| fire_pending_timer(&timers, id))
    });
    let dispatch_timers = Arc::clone(&timers);
    window.set_method(
        "__dispatchTimer",
        Arc::new(move |arguments| {
            if let Some(Value::Number(id)) = arguments.first()
                && id.is_finite()
                && *id >= 1.0
                && *id <= i32::MAX as f64
            {
                fire_pending_timer(&dispatch_timers, *id as u32);
            }
            Ok(Value::Undefined)
        }),
    );
    let clear_all_timers = Arc::clone(&timers);
    window.set_method(
        "__clearTimers",
        Arc::new(move |_| {
            let mut timers = clear_all_timers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            for (_, timer) in timers.drain() {
                timer.cancelled.store(true, Ordering::Release);
            }
            Ok(Value::Undefined)
        }),
    );
    window.set_method(
        "setTimeout",
        Arc::new(move |arguments| {
            let handler = match arguments.first() {
                Some(Value::Function(callback)) => TimerHandler::Callback(callback.clone()),
                Some(handler) => TimerHandler::Source(handler.to_string()),
                None => return Err(JsError::runtime("setTimeout() requires a handler")),
            };
            let delay = webidl_long(arguments.get(1).unwrap_or(&Value::Undefined))?.max(0);
            let nesting_level = nesting_context_for_set.load(Ordering::Acquire);
            let delay = if nesting_level > 5 && delay < 4 {
                4
            } else {
                delay
            };
            let delay_ms = delay as u64;
            let callback_arguments = arguments.get(2..).unwrap_or_default().to_vec();
            let mut active_timers = timers_for_set
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let id = loop {
                let candidate =
                    next_timer_id_for_set.fetch_add(1, Ordering::Relaxed) % i32::MAX as u32 + 1;
                if !active_timers.contains_key(&candidate) {
                    break candidate;
                }
            };
            let cancelled = Arc::new(AtomicBool::new(false));
            active_timers.insert(
                id,
                PendingTimer {
                    cancelled: Arc::clone(&cancelled),
                    handler,
                    arguments: callback_arguments,
                    nesting_level: nesting_level + 1,
                    nesting_context: Arc::clone(&nesting_context_for_set),
                    evaluator: evaluator.clone(),
                    receiver: timer_receiver.clone(),
                },
            );
            drop(active_timers);
            let job = TimerJob {
                deadline: Instant::now() + Duration::from_millis(delay_ms),
                id,
                cancelled,
                dispatcher: Arc::clone(&timer_dispatcher),
            };
            if timer_scheduler().send(job).is_err() {
                timers_for_set
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id);
                return Err(JsError::runtime("window timer scheduler is unavailable"));
            }
            Ok(Value::Number(f64::from(id)))
        }),
    );
    let timers_for_clear = Arc::clone(&timers);
    window.set_method(
        "clearTimeout",
        Arc::new(move |arguments| {
            let id = webidl_long(arguments.first().unwrap_or(&Value::Undefined))?;
            if id > 0 {
                if let Some(timer) = timers_for_clear
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&(id as u32))
                {
                    timer.cancelled.store(true, Ordering::Release);
                }
            }
            Ok(Value::Undefined)
        }),
    );
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

    if let Some(timeout) = window.get("setTimeout") {
        realm.register_global_value("setTimeout", timeout)?;
    }
    if let Some(clear_timeout) = window.get("clearTimeout") {
        realm.register_global_value("clearTimeout", clear_timeout)?;
    }
    realm.register_global_value("window", Value::Object(window.clone()))?;
    realm.register_global_value("self", Value::Object(window.clone()))?;
    realm.register_global_value("customElements", Value::Object(custom_elements))?;
    realm.register_global_value("navigator", Value::Object(navigator))?;
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
    register_window_with_url_and_device_pixel_ratio_and_timer_dispatcher(
        realm,
        url,
        device_pixel_ratio,
        None,
    )
}

/// Registers window globals with a host timer-task dispatcher.
///
/// `dispatcher` receives each timer ID after its delay so the host can enqueue
/// it on the owning page event loop. Pass `None` to execute timers on the
/// built-in serialized timer task thread.
pub fn register_window_with_url_and_device_pixel_ratio_and_timer_dispatcher(
    realm: &mut Realm,
    url: &str,
    device_pixel_ratio: f64,
    dispatcher: Option<TimerDispatcher>,
) -> JsResult<Object> {
    if !device_pixel_ratio.is_finite() || device_pixel_ratio <= 0.0 {
        return Err(JsError::runtime(
            "window.devicePixelRatio must be finite and greater than zero",
        ));
    }
    register_window_object(realm, url, device_pixel_ratio, dispatcher)
}

/// Runs the timer associated with `id` in `window`, if it is still active.
///
/// Hosts call this from their page task loop after the timer dispatcher
/// reports `id`. Cancelled or already-fired IDs are ignored.
pub fn dispatch_window_timer(window: &Object, id: u32) -> JsResult<()> {
    if let Some(Value::Function(dispatch)) = window.get("__dispatchTimer") {
        dispatch.call(&[Value::Number(f64::from(id))])?;
    }
    Ok(())
}

/// Cancels all active timers associated with `window`.
///
/// Browser hosts call this when a document session is discarded so callbacks
/// cannot keep the old document alive or run after navigation.
pub fn clear_window_timers(window: &Object) {
    if let Some(Value::Function(clear)) = window.get("__clearTimers")
        && let Err(error) = clear.call(&[])
    {
        log::warn!("could not clear timers while discarding a window: {error}");
    }
}
