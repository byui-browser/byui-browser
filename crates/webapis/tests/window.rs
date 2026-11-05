use html::parse_raw_html;
use js::{Realm, Value};
use security::{Origin, RendererSecurityContext, StoragePermission};
use webapis::{
    Document, register_document, register_window, register_window_with_security_context,
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
fn window_origin_is_exposed_as_a_replaceable_serialized_origin() {
    let mut realm = Realm::new();
    register_window_with_security_context(
        &mut realm,
        RendererSecurityContext::new(
            Origin::with_port("http", "example.test", 8080),
            StoragePermission::Allowed,
        ),
    )
    .unwrap();

    assert_eq!(
        realm.evaluate_script("window.origin"),
        Ok(Value::String("http://example.test:8080".into()))
    );
    assert_eq!(
        realm.evaluate_script("window.origin = 'https://changed.test'; window.origin"),
        Ok(Value::String("https://changed.test".into()))
    );
}

#[test]
fn registering_document_preserves_window_origin() {
    let mut realm = Realm::new();
    register_window_with_security_context(
        &mut realm,
        RendererSecurityContext::new(
            Origin::with_port("https", "example.test", 443),
            StoragePermission::Allowed,
        ),
    )
    .unwrap();
    register_document(
        &mut realm,
        Document::from_html_document(&parse_raw_html("<main></main>".into())),
    )
    .unwrap();

    assert_eq!(
        realm.evaluate_script("window.origin"),
        Ok(Value::String("https://example.test".into()))
    );
}
