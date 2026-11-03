//! The script-visible `Window` object and its self-reference.

use std::sync::Arc;

use js::{JsResult, Object, Realm, Value};

/// Registers the script-visible window globals.
///
/// The current implementation exposes the `window` object and the global
/// `self` alias. Their `self()` method returns the same object identity,
/// which is the supported subset of the Window self-reference API. Screen,
/// navigation, and other window-management APIs are not implemented yet.
pub fn register_window(realm: &mut Realm) -> JsResult<()> {
    let window = Object::new();
    let self_window = window.clone();
    window.set_method(
        "self",
        Arc::new(move |_| Ok(Value::Object(self_window.clone()))),
    );

    let window_value = Value::Object(window);
    realm.register_global_value("window", window_value.clone())?;
    realm.register_global_value("self", window_value)
}

#[cfg(test)]
mod tests {
    use js::{Realm, Value};

    use super::register_window;

    #[test]
    fn registration_exposes_window_as_an_object() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert!(matches!(
            realm.evaluate_script("window"),
            Ok(Value::Object(_))
        ));
    }

    #[test]
    fn window_self_returns_the_same_object() {
        let mut realm = Realm::new();
        register_window(&mut realm).unwrap();

        assert_eq!(
            realm.evaluate_script("window.self()"),
            realm.evaluate_script("window")
        );
        assert_eq!(
            realm.evaluate_script("self"),
            realm.evaluate_script("window")
        );
    }

    #[test]
    fn each_realm_gets_a_distinct_window_object() {
        let mut first = Realm::new();
        let mut second = Realm::new();
        register_window(&mut first).unwrap();
        register_window(&mut second).unwrap();

        let Value::Object(first_window) = first.evaluate_script("window").unwrap() else {
            panic!("window should be an object");
        };
        let Value::Object(second_window) = second.evaluate_script("window").unwrap() else {
            panic!("window should be an object");
        };

        assert_ne!(first_window, second_window);
    }
}
