use js::{JsError, Realm, Value};
use security::{Origin, RendererSecurityContext, StoragePermission};
use std::{thread, time::Duration};
use webapis::{
    Document, dispatch_window_keydown, register_document, register_window,
    register_window_with_security_context, register_window_with_url,
    register_window_with_url_and_device_pixel_ratio, report_window_error,
    run_window_hashchange_events, run_window_messages,
};

#[test]
fn window_onkeydown_receives_keyboard_event_metadata_and_can_cancel() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    realm
        .evaluate_script(
            "var seen = null; window.onkeydown = (event) => { seen = [
             this === window, event.type, event.key, event.code, event.location,
             event.ctrlKey, event.shiftKey, event.altKey, event.metaKey,
             event.repeat, event.bubbles, event.cancelable]; return false; }",
        )
        .unwrap();

    assert_eq!(
        dispatch_window_keydown(&window, "A", "KeyA", 0, true, false, true, false, true),
        Ok(false)
    );
    assert_eq!(
        realm.evaluate_script(
            "seen.length === 12 && seen[0] && seen[1] === 'keydown' && seen[2] === 'A' &&
             seen[3] === 'KeyA' && seen[4] === 0 && seen[5] && !seen[6] && seen[7] &&
             !seen[8] && seen[9] && seen[10] && seen[11]"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_onkeydown_receives_bubbling_events_from_the_focused_element() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    register_document(
        &mut realm,
        Document::from_html_document(&html::parse_raw_html(
            "<button id='target'></button>".to_owned(),
        )),
    )
    .unwrap();
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    realm
        .evaluate_script(
            "var order = []; var target = document.getElementById('target');
             target.addEventListener('keydown', () => order.push('target'));
             document.addEventListener('keydown', () => order.push('document'));
             window.onkeydown = () => order.push('window'); target.focus();",
        )
        .unwrap();
    assert_eq!(
        dispatch_window_keydown(
            &window, "Enter", "Enter", 0, false, false, false, false, false
        ),
        Ok(true)
    );
    assert_eq!(
        realm.evaluate_script(
            "order.length === 3 && order[0] === 'target' && order[1] === 'document' &&
             order[2] === 'window'",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn keyboard_event_constructor_exposes_ui_events_defaults_and_modifier_state() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    assert_eq!(
        realm.evaluate_script(
            "var event = KeyboardEvent('keydown', {key: 'A', code: 'KeyA',
             ctrlKey: true, repeat: true});
             event.type === 'keydown' && event.key === 'A' && event.code === 'KeyA' &&
             event.ctrlKey && event.repeat && event.getModifierState('Control') &&
             !event.getModifierState('Shift') && event.location === 0 && event.detail === 0"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_onerror_receives_legacy_error_arguments_and_can_cancel_reporting() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/page").unwrap();
    realm
        .evaluate_script(
            "var observed = []; window.onerror = (message, source, line, column, error) => { \
             observed = [message, source, line, column, error.name, error.message]; return true; }; \
             window.addEventListener('error', (event) => { observed.push(event.type + ':' + event.message); });",
        )
        .unwrap();

    let mut error = JsError::runtime("boom");
    error.location = Some(js::bytecode::SourceLocation {
        offset: 0,
        line: 3,
        column: 7,
    });
    assert_eq!(
        report_window_error(&window, &error, "https://example.test/page.js"),
        Ok(Value::Boolean(false))
    );
    assert_eq!(
        realm.evaluate_script(
            "observed.length === 7 && observed[0] === 'boom' && \
             observed[1] === 'https://example.test/page.js' && observed[2] === 3 && \
             observed[3] === 7 && observed[4] === 'Error' && observed[5] === 'boom' && \
             observed[6] === 'error:boom'"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn post_message_queues_a_cloned_message_event() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/page").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var received = null; var source = null; var origin = null; \
             window.addEventListener('message', (event) => { received = event.data; source = event.source; origin = event.origin; }); \
             var payload = {nested: {value: 1}}; window.postMessage(payload, '/'); \
             received === null"
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(run_window_messages(&window), Ok(1));
    assert_eq!(
        realm.evaluate_script(
            "received !== null && received !== payload && received.nested.value === 1 && \
             source === window && origin === 'https://example.test'"
        ),
        Ok(Value::Boolean(true))
    );
    realm.evaluate_script("payload.nested.value = 9").unwrap();
    assert_eq!(
        realm.evaluate_script("received.nested.value === 1"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn post_message_respects_target_origin_and_options() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/page").unwrap();

    realm
        .evaluate_script(
            "var calls = 0; window.addEventListener('message', () => calls++); \
             window.postMessage('blocked', 'https://other.test'); \
             window.postMessage('allowed', {targetOrigin: 'https://example.test'}); \
             window.postMessage('wildcard', '*')",
        )
        .unwrap();
    assert_eq!(run_window_messages(&window), Ok(2));
    assert_eq!(realm.evaluate_script("calls"), Ok(Value::Number(2.0)));
}

#[test]
fn onmessage_is_an_event_handler_with_window_this_and_event_metadata() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/page").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var seen = null; var handlerCalls = 0; \
             function rememberMessage(event) { \
                 handlerCalls = handlerCalls + 1; \
                 seen = {thisValue: this, type: event.type, data: event.data, \
                         origin: event.origin, source: event.source, target: event.target, \
                         currentTarget: event.currentTarget, bubbles: event.bubbles, \
                         cancelable: event.cancelable}; \
             } \
             window.onmessage = rememberMessage; \
             window.postMessage({value: 1}, '/'); seen === null"
        ),
        Ok(Value::Boolean(true))
    );

    assert_eq!(run_window_messages(&window), Ok(1));
    assert_eq!(
        realm.evaluate_script(
            "handlerCalls === 1 && seen.thisValue === window && seen.type === 'message' && \
             seen.data.value === 1 && seen.origin === 'https://example.test' && \
             seen.source === window && seen.target === window && \
             seen.currentTarget === window && !seen.bubbles && !seen.cancelable"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn onmessage_can_be_removed_and_runs_with_event_listeners() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/page").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var order = []; \
             window.addEventListener('message', () => order.push('listener')); \
             window.onmessage = () => order.push('handler'); \
             window.postMessage('first', '/');"
        ),
        Ok(Value::Undefined)
    );
    assert_eq!(run_window_messages(&window), Ok(1));
    assert_eq!(
        realm.evaluate_script(
            "order.length === 2 && order[0] === 'listener' && order[1] === 'handler'"
        ),
        Ok(Value::Boolean(true))
    );

    realm
        .evaluate_script("window.onmessage = null; window.postMessage('second', '/');")
        .unwrap();
    assert_eq!(run_window_messages(&window), Ok(1));
    assert_eq!(
        realm.evaluate_script(
            "order.length === 3 && order[0] === 'listener' && order[1] === 'handler' && order[2] === 'listener'",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn message_dispatch_honors_once_and_stop_immediate_propagation() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/page").unwrap();

    realm
        .evaluate_script(
            "var calls = 0; var handlerCalls = 0; \
             window.addEventListener('message', () => calls++, {once: true}); \
             window.onmessage = () => handlerCalls++; \
             window.postMessage('first', '/'); window.postMessage('second', '/')",
        )
        .unwrap();
    assert_eq!(run_window_messages(&window), Ok(2));
    assert_eq!(
        realm.evaluate_script("calls === 1 && handlerCalls === 2"),
        Ok(Value::Boolean(true))
    );

    realm
        .evaluate_script(
            "window.addEventListener('message', (event) => event.stopImmediatePropagation()); \
             window.postMessage('blocked', '/')",
        )
        .unwrap();
    assert_eq!(run_window_messages(&window), Ok(1));
    assert_eq!(
        realm.evaluate_script("handlerCalls === 2"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_self_returns_the_current_window_object() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.self() === window"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn global_self_aliases_window() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("self === window && self.self() === window"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_onload_and_load_listeners_receive_the_window_load_event_in_order() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    webapis::register_document(&mut realm, webapis::Document::new()).unwrap();

    assert_eq!(
        realm.evaluate_script("window.onload === null"),
        Ok(Value::Boolean(true))
    );
    realm
        .evaluate_script(
            "var order = ''; \
             function first(event) { order = order + 'a'; \
               window.firstEventMatches = this === window && event.type === 'load' && \
                 event.target === document && event.currentTarget === window && \
                 event.bubbles === false && event.cancelable === false; } \
             function handler(event) { order = order + 'b'; } \
             function last(event) { order = order + 'c'; } \
             window.addEventListener('load', first); window.onload = handler; \
             window.addEventListener('load', last); window.onload === handler",
        )
        .unwrap();

    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    webapis::dispatch_window_load(&window).unwrap();

    assert_eq!(
        realm.evaluate_script("order === 'abc' && window.firstEventMatches"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("window.onload = null; window.onload === null"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_add_event_listener_dispatches_and_removes_callbacks() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var calls = ''; \
             function listener(event) { calls += event.type + ':' + (event.target === window); } \
             window.addEventListener('ready', listener); \
             window.addEventListener('ready', listener); \
             window.dispatchEvent({type: 'ready'}); \
             window.removeEventListener('ready', listener); \
             window.dispatchEvent({type: 'ready'}); \
             calls",
        ),
        Ok(Value::String("ready:true".into()))
    );
}

#[test]
fn window_onclick_is_an_event_handler_attribute() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.onclick === null"),
        Ok(Value::Boolean(true))
    );
    realm
        .evaluate_script(
            "var order = []; var seen = {}; \
             function listener() { order.push('listener'); } \
             function handler(event) { order.push('handler'); \
               seen.thisValue = this; seen.type = event.type; \
               seen.target = event.target; seen.currentTarget = event.currentTarget; \
               seen.bubbles = event.bubbles; seen.cancelable = event.cancelable; } \
             window.addEventListener('click', listener); window.onclick = handler; \
             window.onclick === handler && window.dispatchEvent({type: 'click', bubbles: true, cancelable: true})",
        )
        .unwrap();

    assert_eq!(
        realm.evaluate_script(
            "order.length === 2 && order[0] === 'listener' && order[1] === 'handler' && \
             seen.thisValue === window && seen.type === 'click' && \
             seen.target === window && seen.currentTarget === window && \
             seen.bubbles && seen.cancelable",
        ),
        Ok(Value::Boolean(true))
    );

    realm
        .evaluate_script(
            "window.onclick = null; window.dispatchEvent({type: 'click'}); window.onclick === null",
        )
        .unwrap();
    assert_eq!(
        realm.evaluate_script("order.length === 3 && order[2] === 'listener'"),
        Ok(Value::Boolean(true))
    );
    assert!(realm.evaluate_script("window.onclick = 1").is_err());
}

#[test]
fn clearing_window_onload_deactivates_the_handler() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; function handler(event) { calls = calls + 1; } \
             window.onload = handler; window.onload = null",
        )
        .unwrap();

    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    webapis::dispatch_window_load(&window).unwrap();

    assert_eq!(
        realm.evaluate_script("calls === 0 && window.onload === null"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_add_event_listener_supports_once_objects_and_abort_signals() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var calls = 0; var signal = {aborted: true}; \
             var objectListener = {}; function objectHandler(event) { calls += 1; } \
             objectListener.handleEvent = objectHandler; \
             function abortedHandler() { calls += 100; } \
             window.addEventListener('once', objectListener, {once: true}); \
             window.addEventListener('aborted', abortedHandler, {signal: signal}); \
             window.dispatchEvent({type: 'once'}); window.dispatchEvent({type: 'once'}); \
             window.dispatchEvent({type: 'aborted'}); calls",
        ),
        Ok(Value::Number(1.0))
    );
}

#[test]
fn window_self_accepts_extra_arguments_without_changing_the_result() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.self(1, 'ignored') === window"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_dispatch_event_runs_listeners_and_returns_cancellation_state() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    register_document(&mut realm, Document::new()).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var order = []; \
             window.addEventListener('ready', (event) => { order.push('capture:' + event.eventPhase); }, true); \
             window.addEventListener('ready', (event) => { order.push('bubble:' + (event.target === window)); }); \
             var event = Event('ready', {cancelable: true}); \
             window.dispatchEvent(event) && order.length === 2 && order[0] === 'capture:2' && order[1] === 'bubble:true' && \
             event.currentTarget === null && event.eventPhase === 0"
        ),
        Ok(Value::Boolean(true))
    );

    assert_eq!(
        realm.evaluate_script(
            "window.addEventListener('cancel', (event) => event.preventDefault()); \
             window.dispatchEvent(Event('cancel', {cancelable: true})) === false"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_dispatch_event_honors_once_and_abort_signal() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    register_document(&mut realm, Document::new()).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var calls = 0; var controller = AbortController(); \
             window.addEventListener('ready', () => calls++, {once: true}); \
             window.addEventListener('ready', () => calls += 10, {signal: controller.signal}); \
             controller.abort(); window.dispatchEvent(Event('ready')); window.dispatchEvent(Event('ready')); calls === 1"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_navigator_returns_the_global_navigator_object() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.navigator() === navigator"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_navigator_returns_a_stable_object() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.navigator() === window.navigator()"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_navigator_accepts_extra_arguments_without_changing_the_result() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.navigator('ignored') === navigator"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_get_selection_is_null_without_an_associated_document() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.getSelection() === null"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_get_selection_returns_the_document_singleton_and_initial_empty_state() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    register_document(&mut realm, Document::new()).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.getSelection() === document.getSelection() && \
             getSelection() === window.getSelection() && \
             window.getSelection().anchorNode === null && \
             window.getSelection().focusNode === null && \
             window.getSelection().anchorOffset === 0 && \
             window.getSelection().focusOffset === 0 && \
             window.getSelection().isCollapsed === true && \
             window.getSelection().rangeCount === 0 && \
             window.getSelection().type === 'None' && \
             window.getSelection().direction === 'none' && \
             window.getSelection().toString() === '' && \
             window.getSelection().getComposedRanges().length === 0 && \
             window.getSelection().containsNode(null) === false"
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_performance_exposes_a_stable_monotonic_clock() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "performance === window.performance && \
             performance.timeOrigin > 0 && \
             performance.now() >= 0",
        ),
        Ok(Value::Boolean(true))
    );

    let first = realm.evaluate_script("performance.now()").unwrap();
    thread::sleep(Duration::from_millis(1));
    let second = realm.evaluate_script("window.performance.now()").unwrap();
    let (Value::Number(first), Value::Number(second)) = (first, second) else {
        panic!("performance.now() should return numbers");
    };
    assert!(second >= first);
}

#[test]
fn window_performance_object_and_time_origin_are_read_only() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var original = window.performance; \
             window.performance = null; \
             performance.timeOrigin = 0; \
             window.performance === original && performance.timeOrigin > 0",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_origin_uses_the_renderer_security_context() {
    let mut realm = Realm::new();
    register_window_with_security_context(
        &mut realm,
        RendererSecurityContext::new(
            Origin::with_port("https", "example.test", 8443),
            StoragePermission::Allowed,
        ),
    )
    .unwrap();

    assert_eq!(
        realm.evaluate_script("window.origin === 'https://example.test:8443'"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_scroll_x_exposes_the_initial_horizontal_offset() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.scrollX === 0 && window.scrollX === window.scrollX"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_scroll_x_is_read_only() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.scrollX = 12; window.scrollX === 0"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_scroll_y_exposes_the_initial_vertical_offset() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.scrollY === 0 && window.pageYOffset === 0 && window.scrollY === window.scrollY",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_scroll_y_is_read_only() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.scrollY = 12; window.pageYOffset = 12; window.scrollY === 0 && window.pageYOffset === 0",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn device_pixel_ratio_exposes_the_output_scale_and_is_replaceable() {
    let mut realm = Realm::new();
    register_window_with_url_and_device_pixel_ratio(&mut realm, "https://example.test/", 1.5)
        .unwrap();

    assert_eq!(
        realm.evaluate_script("window.devicePixelRatio === 1.5"),
        Ok(Value::Boolean(true))
    );
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert!(window.update_replaceable_read_only("devicePixelRatio", Value::Number(1.75),));
    assert_eq!(
        realm.evaluate_script("window.devicePixelRatio === 1.75"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("window.devicePixelRatio = 2; window.devicePixelRatio === 2",),
        Ok(Value::Boolean(true))
    );
    assert!(!window.update_replaceable_read_only("devicePixelRatio", Value::Number(2.5),));
    assert_eq!(
        realm.evaluate_script("window.devicePixelRatio === 2"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn device_pixel_ratio_requires_a_finite_positive_scale() {
    let mut realm = Realm::new();

    assert!(
        register_window_with_url_and_device_pixel_ratio(&mut realm, "https://example.test/", 0.0,)
            .is_err()
    );
}

#[test]
fn window_history_is_stable_and_starts_at_one_entry() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "history === window.history && window.history === window.history && \
             window.history.length === 1 && window.history.state === null",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn history_push_replace_and_navigation_update_state() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.history.pushState('first', ''); \
             window.history.pushState('second', ''); \
             window.history.back(); window.history.state === 'first'",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script(
            "window.history.replaceState('updated', ''); \
             window.history.forward(); window.history.state === 'second'",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("window.history.go(-2); window.history.state === null"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn pushing_state_discards_forward_entries() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.history.pushState('first', ''); \
             window.history.pushState('second', ''); window.history.back(); \
             window.history.pushState('replacement', ''); \
             window.history.length === 3 && window.history.forward(); \
             window.history.state === 'replacement'",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn history_resolves_same_origin_urls_and_dispatches_popstate() {
    let mut realm = Realm::new();
    register_window_with_url(&mut realm, "https://example.test/start").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var seen = 'unset'; \
             function remember(event) { seen = event.state; } \
             window.addEventListener('popstate', remember); \
             window.history.pushState('next', '', '/next'); \
             window.location().href === 'https://example.test/next' && \
             window.history.state === 'next'",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script(
            "window.history.back(); \
             window.location().href === 'https://example.test/start' && seen === null",
        ),
        Ok(Value::Boolean(true))
    );
    assert!(
        realm
            .evaluate_script("window.history.pushState(null, '', 'https://other.test/blocked')")
            .is_err()
    );
    assert_eq!(
        realm.evaluate_script(
            "window.history.pushState('same', '', null); window.location().href === 'https://example.test/start'",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_onpopstate_is_an_event_handler_attribute() {
    let mut realm = Realm::new();
    register_window_with_url(&mut realm, "https://example.test/start").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.onpopstate === null && \
             window.history.pushState('next', '', '/next'); \
             var calls = 0; var seen_state; var seen_this; \
             function remember(event) { calls++; seen_state = event.state; seen_this = this; } \
             window.onpopstate = remember; \
             window.history.back(); \
             calls === 1 && seen_state === null && seen_this === window && \
             window.onpopstate !== null",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script(
            "window.onpopstate = null; window.history.forward(); calls === 1 && window.onpopstate === null",
        ),
        Ok(Value::Boolean(true))
    );
    assert!(realm.evaluate_script("window.onpopstate = 1").is_err());
}

#[test]
fn window_onpopstate_runs_in_event_listener_order() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.history.pushState('next', ''); var order = ''; \
             function listener() { order += 'listener'; } \
             function handler() { order += 'handler'; } \
             window.addEventListener('popstate', listener); \
             window.onpopstate = handler; \
             window.history.back(); order === 'listenerhandler'",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_onhashchange_reports_fragment_urls_during_history_traversal() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/start").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var calls = 0; var old_url; var new_url; var seen_this; var flags; \
             function remember_hash(event) { calls++; old_url = event.oldURL; \
               new_url = event.newURL; seen_this = this; flags = event.bubbles + ':' + event.cancelable; } \
             window.onhashchange = remember_hash; \
             window.history.pushState(null, '', '#part'); window.history.back(); \
             calls === 0",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(run_window_hashchange_events(&window), Ok(1));
    assert_eq!(
        realm.evaluate_script(
            "calls === 1 && old_url === 'https://example.test/start#part' && \
             new_url === 'https://example.test/start' && seen_this === window && flags === 'false:false'",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn window_onhashchange_is_an_event_handler_attribute_and_only_fires_for_fragment_changes() {
    let mut realm = Realm::new();
    let window = register_window_with_url(&mut realm, "https://example.test/start").unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var calls = 0; function count_hash() { calls++; } \
             window.addEventListener('hashchange', count_hash); \
             window.history.pushState(null, '', '/next'); window.history.back(); calls === 0 && \
             window.onhashchange === null",
        ),
        Ok(Value::Boolean(true))
    );
    assert!(realm.evaluate_script("window.onhashchange = 1").is_err());
    assert_eq!(
        realm.evaluate_script(
            "function count_hash_attribute() { calls++; } window.onhashchange = count_hash_attribute; \
             window.history.pushState(null, '', '#x'); \
             window.history.back(); calls === 0; window.onhashchange = null; window.history.forward(); calls === 0",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(run_window_hashchange_events(&window), Ok(2));
    assert_eq!(
        realm.evaluate_script("calls === 2"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn history_state_is_structured_cloned() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var original = {nested: {value: 1}}; \
             window.history.pushState(original, ''); \
             original.nested.value = 2; \
             var first_read = window.history.state; first_read.nested.value = 3; \
             window.history.state.nested.value === 1",
        ),
        Ok(Value::Boolean(true))
    );

    assert_eq!(
        realm.evaluate_script(
            "window.history.pushState({nested: {value: 2}}, ''); \
             function mutate(event) { event.state.nested.value = 9; } \
             window.onpopstate = mutate; window.history.back(); \
             window.history.state.nested.value === 1",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn structured_clone_deep_copies_objects_arrays_cycles_and_shared_references() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var child = {value: 1}; var original = {left: child, right: child}; \
             original.self = original; original.list = [child, 2]; \
             var copy = window.structuredClone(original); \
             copy !== original && copy.self === copy && copy.left === copy.right && \
             copy.left !== child && copy.list[0] === copy.left && \
             structuredClone === window.structuredClone",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("child.value = 3; copy.left.value === 1"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn structured_clone_accepts_empty_transfer_list_and_rejects_functions() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "function callback() {} var cloneError = false; \
             try { structuredClone({callback: callback}); } \
             catch (error) { cloneError = error.name === 'DataCloneError'; } \
             structuredClone({value: 1}, {transfer: []}).value === 1 && cloneError",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn structured_clone_reports_unsupported_transfers_as_data_clone_errors() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "try { structuredClone({}, {transfer: [{}]}); } \
             catch (error) { error.name === 'DataCloneError'; }",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn inner_width_is_zero_without_a_viewport() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.innerWidth"),
        Ok(Value::Number(0.0))
    );
}

#[test]
fn inner_height_is_zero_without_a_viewport() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.innerHeight"),
        Ok(Value::Number(0.0))
    );
}

#[test]
fn inner_width_reports_the_viewport_width_in_css_pixels() {
    let mut realm = Realm::new();
    webapis::register_window_with_security_context_and_viewport(
        &mut realm,
        RendererSecurityContext::new(
            Origin::new("https", "example.com", None),
            StoragePermission::Allowed,
        ),
        webapis::ViewportSize {
            width: 800,
            height: 600,
        },
    )
    .unwrap();

    assert_eq!(
        realm.evaluate_script("window.innerWidth"),
        Ok(Value::Number(800.0))
    );
    assert_eq!(
        realm.evaluate_script("innerWidth"),
        Ok(Value::Number(800.0))
    );
    assert_eq!(
        realm.evaluate_script("window.innerHeight"),
        Ok(Value::Number(600.0))
    );
    assert_eq!(
        realm.evaluate_script("innerHeight"),
        Ok(Value::Number(600.0))
    );
    assert_eq!(
        realm.evaluate_script("window.origin"),
        Ok(Value::String("https://example.com".into()))
    );
}

#[test]
fn inner_width_is_replaceable_like_the_cssom_view_attribute() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.innerWidth = 42; window.innerWidth"),
        Ok(Value::Number(42.0))
    );
}

#[test]
fn inner_height_is_replaceable_like_the_cssom_view_attribute() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.innerHeight = 42; window.innerHeight"),
        Ok(Value::Number(42.0))
    );
}

#[test]
fn match_media_exposes_window_management_and_viewport_features() {
    let mut realm = Realm::new();
    webapis::register_window_with_url_device_pixel_ratio_and_viewport(
        &mut realm,
        "https://example.test/",
        2.0,
        Some(webapis::ViewportSize {
            width: 800,
            height: 600,
        }),
    )
    .unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var a = matchMedia('(min-width: 700px) and (orientation: landscape)'); \
             var b = window.matchMedia('(resolution: 192dpi)'); \
             var c = matchMedia('(display-state: normal) and (resizable: true)'); \
             a.media === '(min-width: 700px) and (orientation: landscape)' && \
             a.matches && b.matches && c.matches && a === matchMedia('(min-width: 700px) and (orientation: landscape)') === false",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn match_media_change_events_update_matches_and_support_legacy_listeners() {
    let mut realm = Realm::new();
    webapis::register_window_with_url_device_pixel_ratio_and_viewport(
        &mut realm,
        "https://example.test/",
        1.0,
        Some(webapis::ViewportSize {
            width: 800,
            height: 600,
        }),
    )
    .unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var list = matchMedia('(min-width: 700px)'); var calls = 0; var seen = false; \
             function listener(event) { calls = calls + 1; seen = event.matches === false && event.media === list.media; } \
             list.addListener(listener); list.onchange = listener; \
             window.__updateMediaQueries(500, 600, 1); \
             list.matches === false && calls === 2 && seen",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script(
            "list.removeListener(listener); window.__updateMediaQueries(800, 600, 1); calls === 3",
        ),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn set_interval_returns_positive_id_passes_arguments_and_repeats() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; var total = 0; \
             function tick(value) { calls = calls + 1; total = total + value; } \
             var id = window.setInterval(tick, 1, 3); \
             id > 0 && setInterval === window.setInterval",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(3));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert!(webapis::run_window_timers(&window).unwrap() >= 1);
    assert_eq!(
        realm.evaluate_script("calls >= 1 && total === calls * 3"),
        Ok(Value::Boolean(true))
    );
    realm.evaluate_script("clearInterval(id)").unwrap();
    thread::sleep(Duration::from_millis(3));
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 0);
}

#[test]
fn set_timeout_fires_once_with_arguments_and_window_receiver() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; var passed = false; \
             function tick(value) { calls = calls + 1; passed = this === window && value === 7; } \
             var id = setTimeout(tick, 1, 7); id > 0",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(3));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 1);
    assert_eq!(
        realm.evaluate_script("calls === 1 && passed"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 0);
}

#[test]
fn clear_timeout_cancels_callback_and_string_handler_runs() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; function cancelled_callback() { calls = 99; } \
             var cancelled = setTimeout(cancelled_callback, 1); \
             clearTimeout(cancelled); setTimeout('calls = calls + 1', 1)",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(3));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 1);
    assert_eq!(realm.evaluate_script("calls"), Ok(Value::Number(1.0)));
}

#[test]
fn queue_microtask_runs_in_fifo_order_and_drains_new_microtasks() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var order = ''; \
             function first() { order = order + 'a'; queueMicrotask(third); } \
             function second() { order = order + 'b'; } \
             function third() { order = order + 'c'; } \
             queueMicrotask(first); window.queueMicrotask(second); \
             order",
        )
        .unwrap();
    assert_eq!(realm.evaluate_script("order"), Ok(Value::String("".into())));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert_eq!(webapis::run_window_microtasks(&window).unwrap(), 3);
    assert_eq!(
        realm.evaluate_script("order"),
        Ok(Value::String("abc".into()))
    );
}

#[test]
fn queue_microtask_requires_a_function_and_returns_undefined() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    assert_eq!(
        realm.evaluate_script("function callback() {} queueMicrotask(callback)"),
        Ok(Value::Undefined)
    );
    assert!(realm.evaluate_script("queueMicrotask(1)").is_err());
}

#[test]
fn timer_checkpoint_runs_microtasks_before_the_next_timer() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var order = ''; \
             function first() { order = order + 'a'; queueMicrotask(micro); } \
             function micro() { order = order + 'm'; } \
             function second() { order = order + 'b'; } \
             setTimeout(first, 0); setTimeout(second, 0)",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(2));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 2);
    assert_eq!(
        realm.evaluate_script("order"),
        Ok(Value::String("amb".into()))
    );
}

#[test]
fn clear_interval_can_be_called_inside_its_callback() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; var id; function tick() { calls = calls + 1; clearInterval(id); } id = setInterval(tick, 1);",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(3));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 1);
    assert_eq!(realm.evaluate_script("calls"), Ok(Value::Number(1.0)));
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 0);
}

#[test]
fn clear_interval_uses_shared_timer_handles_and_returns_undefined() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; function tick() { calls = calls + 1; } \
             var id = setInterval(tick, 1); \
             clearInterval('1') === undefined && clearTimeout === clearInterval",
        )
        .unwrap();

    thread::sleep(Duration::from_millis(3));
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };
    assert_eq!(webapis::run_window_timers(&window).unwrap(), 0);
    assert_eq!(realm.evaluate_script("calls"), Ok(Value::Number(0.0)));

    assert_eq!(
        realm.evaluate_script("clearInterval(undefined)"),
        Ok(Value::Undefined)
    );
}

#[test]
fn request_animation_frame_invokes_callback_once_with_timestamp() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; var timestamp = -1; \
             function tick(now) { calls = calls + 1; timestamp = now; } \
             var id = requestAnimationFrame(tick); \
             id > 0 && requestAnimationFrame === window.requestAnimationFrame",
        )
        .unwrap();
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };

    assert_eq!(webapis::run_window_animation_frames(&window).unwrap(), 1);
    assert_eq!(webapis::run_window_animation_frames(&window).unwrap(), 0);
    assert_eq!(realm.evaluate_script("calls"), Ok(Value::Number(1.0)));
    let timestamp = realm.evaluate_script("timestamp").unwrap();
    assert!(matches!(timestamp, Value::Number(value) if value.is_finite() && value >= 0.0));
}

#[test]
fn request_animation_frame_callbacks_are_ordered_and_defer_new_callbacks() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var order = ''; \
             function first(now) { order = order + 'a'; requestAnimationFrame(third); } \
             function second(now) { order = order + 'b'; } \
             function third() { order = order + 'c'; } \
             requestAnimationFrame(first); requestAnimationFrame(second);",
        )
        .unwrap();
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };

    assert_eq!(webapis::run_window_animation_frames(&window).unwrap(), 2);
    assert_eq!(
        realm.evaluate_script("order"),
        Ok(Value::String("ab".into()))
    );
    assert_eq!(webapis::run_window_animation_frames(&window).unwrap(), 1);
    assert_eq!(
        realm.evaluate_script("order"),
        Ok(Value::String("abc".into()))
    );
}

#[test]
fn cancel_animation_frame_prevents_callback() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "var calls = 0; \
             function tick() { calls = calls + 1; } \
             var id = requestAnimationFrame(tick); \
             cancelAnimationFrame(id);",
        )
        .unwrap();
    let Some(Value::Object(window)) = realm.registered_global_value("window") else {
        panic!("window global should be registered");
    };

    assert_eq!(webapis::run_window_animation_frames(&window).unwrap(), 0);
    assert_eq!(realm.evaluate_script("calls"), Ok(Value::Number(0.0)));
}
