use js::{Realm, Value};
use security::{Origin, RendererSecurityContext, StoragePermission};
use std::thread;
use std::time::{Duration, Instant};
use std::{sync::Arc, sync::mpsc};
use webapis::{
    TimerDispatcher, dispatch_window_timer, register_window, register_window_with_security_context,
    register_window_with_url, register_window_with_url_and_device_pixel_ratio,
    register_window_with_url_and_device_pixel_ratio_and_timer_dispatcher,
};

#[test]
fn set_timeout_runs_function_with_extra_arguments_and_can_be_cancelled() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "function complete(value) { window.completed = value; window.callbackThis = this === window; } \
             function cancelMe() { window.cancelled = true; } \
             window.setTimeout(complete, 0, 37); \
             const cancelled = window.setTimeout(cancelMe, 100); \
             window.clearTimeout(cancelled);",
        )
        .unwrap();

    thread::sleep(Duration::from_millis(30));
    assert_eq!(
        realm.evaluate_script("window.completed"),
        Ok(Value::Number(37.0))
    );
    assert_eq!(
        realm.evaluate_script("window.callbackThis"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("window.cancelled === undefined"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn set_timeout_accepts_string_handlers_and_webidl_delay_conversion() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();
    realm
        .evaluate_script(
            "function delayAsFunction() {} \
             setTimeout('this.stringCompleted = true', '0'); \
             setTimeout('this.functionDelayCompleted = true', delayAsFunction);",
        )
        .unwrap();
    thread::sleep(Duration::from_millis(30));
    assert_eq!(
        realm.evaluate_script("window.stringCompleted"),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("window.functionDelayCompleted"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn set_timeout_waits_for_host_page_task_dispatch() {
    let mut realm = Realm::new();
    let (sender, receiver) = mpsc::channel();
    let dispatcher: TimerDispatcher = Arc::new(move |id| {
        sender.send(id).unwrap();
    });
    let window = register_window_with_url_and_device_pixel_ratio_and_timer_dispatcher(
        &mut realm,
        "about:blank",
        1.0,
        Some(dispatcher),
    )
    .unwrap();
    realm
        .evaluate_script("setTimeout('this.dispatched = true', 0);")
        .unwrap();

    let id = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        realm.evaluate_script("window.dispatched"),
        Ok(Value::Undefined)
    );
    dispatch_window_timer(&window, id).unwrap();
    assert_eq!(
        realm.evaluate_script("window.dispatched"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn nested_set_timeout_clamps_short_delays_after_five_levels() {
    let mut realm = Realm::new();
    let (sender, receiver) = mpsc::channel();
    let dispatcher: TimerDispatcher = Arc::new(move |id| {
        sender.send(id).unwrap();
    });
    let window = register_window_with_url_and_device_pixel_ratio_and_timer_dispatcher(
        &mut realm,
        "about:blank",
        1.0,
        Some(dispatcher),
    )
    .unwrap();
    realm
        .evaluate_script(
            "function repeatTimer(remaining) { \
                 if (remaining > 0) setTimeout(repeatTimer, 0, remaining - 1); \
                 else window.done = true; \
             } \
             setTimeout(repeatTimer, 0, 6);",
        )
        .unwrap();

    let mut id = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    for _ in 0..5 {
        dispatch_window_timer(&window, id).unwrap();
        id = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    let started = Instant::now();
    dispatch_window_timer(&window, id).unwrap();
    receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(3));
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
fn window_self_accepts_extra_arguments_without_changing_the_result() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script("window.self(1, 'ignored') === window"),
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
            "window.history() === window.history() && \
             window.history().length === 1 && window.history().state === null",
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
            "window.history().pushState('first', ''); \
             window.history().pushState('second', ''); \
             window.history().back(); window.history().state === 'first'",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script(
            "window.history().replaceState('updated', ''); \
             window.history().forward(); window.history().state === 'second'",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script("window.history().go(-2); window.history().state === null"),
        Ok(Value::Boolean(true))
    );
}

#[test]
fn pushing_state_discards_forward_entries() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "window.history().pushState('first', ''); \
             window.history().pushState('second', ''); window.history().back(); \
             window.history().pushState('replacement', ''); \
             window.history().length === 3 && window.history().forward(); \
             window.history().state === 'replacement'",
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
             window.history().pushState('next', '', '/next'); \
             window.location().href === 'https://example.test/next' && \
             window.history().state === 'next'",
        ),
        Ok(Value::Boolean(true))
    );
    assert_eq!(
        realm.evaluate_script(
            "window.history().back(); \
             window.location().href === 'https://example.test/start' && seen === null",
        ),
        Ok(Value::Boolean(true))
    );
    assert!(
        realm
            .evaluate_script("window.history().pushState(null, '', 'https://other.test/blocked')")
            .is_err()
    );
}

#[test]
fn history_state_is_structured_cloned() {
    let mut realm = Realm::new();
    register_window(&mut realm).unwrap();

    assert_eq!(
        realm.evaluate_script(
            "var original = {nested: {value: 1}}; \
             window.history().pushState(original, ''); \
             original.nested.value = 2; \
             window.history().state.nested.value === 1",
        ),
        Ok(Value::Boolean(true))
    );
}
