use js::{Realm, Value};
use security::{Origin, RendererSecurityContext, StoragePermission};
use std::{thread, time::Duration};
use webapis::{
    register_window, register_window_with_security_context, register_window_with_url,
    register_window_with_url_and_device_pixel_ratio,
};

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
