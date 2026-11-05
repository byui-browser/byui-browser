//! The script-visible `Window` object and its standard object references.

use std::sync::Arc;

use js::{JsResult, Object, Realm, Value};
use security::{Origin, RendererSecurityContext};

/// Registers the script-visible window globals.
///
/// The current implementation exposes the `window` object and the global
/// `self` alias. Their `self()` method returns the same object identity,
/// which is the supported subset of the Window self-reference API.
///
/// `window.navigator()` returns one stable Navigator object, and the global
/// `navigator` name refers to that same object. The JavaScript engine's host
/// binding convention currently exposes the Window attribute as a callable
/// method, so callers use `window.navigator()` rather than the browser's
/// usual `window.navigator` property. The Navigator object is intentionally
/// partial: no Navigator members are defined yet.
pub fn register_window(realm: &mut Realm) -> JsResult<()> {
    register_window_with_origin(realm, Origin::placeholder())
}

/// Registers the script-visible Window globals for a renderer security context.
///
/// `window.origin` and the global `origin` value are serialized from the
/// supplied context's immutable origin. Use this entry point when booting a
/// real document; [`register_window`] remains a placeholder-only convenience
/// for callers that do not have a navigation context.
pub fn register_window_with_security_context(
    realm: &mut Realm,
    security_context: RendererSecurityContext,
) -> JsResult<()> {
    register_window_with_origin(realm, security_context.origin().clone())
}

fn register_window_with_origin(realm: &mut Realm, origin: Origin) -> JsResult<()> {
    let window = Object::new();
    let self_window = window.clone();
    let navigator = Object::new();
    let navigator_for_window = navigator.clone();
    let serialized_origin = origin.ascii_serialization();
    window.set_method(
        "self",
        Arc::new(move |_| Ok(Value::Object(self_window.clone()))),
    );
    window.set_method(
        "navigator",
        Arc::new(move |_| Ok(Value::Object(navigator_for_window.clone()))),
    );
    window.set("origin", Value::String(serialized_origin.clone()));

    let window_value = Value::Object(window);
    realm.register_global_value("window", window_value.clone())?;
    realm.register_global_value("self", window_value)?;
    realm.register_global_value("navigator", Value::Object(navigator))?;
    realm.register_global_value("origin", Value::String(serialized_origin))
}

#[cfg(test)]
mod tests {
    use js::{Realm, Value};
    use security::{Origin, RendererSecurityContext, StoragePermission};

    use super::{register_window, register_window_with_security_context};

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

    #[test]
    fn window_origin_uses_the_security_context_origin() {
        let mut realm = Realm::new();
        register_window_with_security_context(
            &mut realm,
            RendererSecurityContext::new(
                Origin::with_port("https", "Example.COM", 8443),
                StoragePermission::Allowed,
            ),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script("window.origin"),
            Ok(Value::String("https://example.com:8443".into()))
        );
        assert_eq!(
            realm.evaluate_script("origin === window.origin"),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn window_origin_omits_default_ports() {
        let mut realm = Realm::new();
        register_window_with_security_context(
            &mut realm,
            RendererSecurityContext::new(
                Origin::with_port("https", "example.com", 443),
                StoragePermission::Allowed,
            ),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script("window.origin"),
            Ok(Value::String("https://example.com".into()))
        );
    }

    #[test]
    fn opaque_window_origin_serializes_as_null() {
        let mut realm = Realm::new();
        register_window_with_security_context(
            &mut realm,
            RendererSecurityContext::new(
                Origin::new("opaque", "", None),
                StoragePermission::Denied,
            ),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script("window.origin"),
            Ok(Value::String("null".into()))
        );
    }
}
