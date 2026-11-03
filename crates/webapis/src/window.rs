//! The JavaScript [`window`](https://developer.mozilla.org/en-US/docs/Web/API/Window)
//! global object.

use js::{JsResult, Object, Realm, Value};

/// Registers the page's `window` object in `realm`.
///
/// A fresh object is created for each realm, so separate pages cannot observe
/// or mutate one another's window state. Window members are registered by
/// their respective Web API bindings as those APIs are implemented.
pub fn register_window(realm: &mut Realm) -> JsResult<()> {
    realm.register_global_value("window", Value::Object(Object::new()))
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
