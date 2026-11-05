use js::{Realm, Value};
use webapis::{register_window, register_window_with_url};

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
