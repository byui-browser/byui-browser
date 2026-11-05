//! The script-visible `Window` object and its standard object references.

use std::sync::Arc;

use crate::custom_elements::CustomElementRegistry;
use js::{JsResult, Object, Realm, Value};
use security::{Origin, RendererSecurityContext};

/// Registers the script-visible window globals.
///
/// The current implementation exposes the `window` object and the global
/// `self` alias. Their `self()` method returns the same object identity,
/// which is the supported subset of the Window self-reference API.
///
/// It exposes the document's `customElements` registry. Custom element
/// construction and lifecycle support cover the current single-document DOM
/// subset; scoped registries and native promises are not implemented yet.
///
/// `window.navigator()` returns one stable Navigator object, and the global
/// `navigator` name refers to that same object. The JavaScript engine's host
/// binding convention currently exposes the Window attribute as a callable
/// method, so callers use `window.navigator()` rather than the browser's
/// usual `window.navigator` property. The Navigator object is intentionally
/// partial: no Navigator members are defined yet.
pub fn register_window(realm: &mut Realm) -> JsResult<()> {
    register_window_with_origin(realm, Origin::placeholder(), None)
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
    register_window_with_origin(realm, security_context.origin().clone(), None)
}

/// The viewport dimensions exposed to a script-visible Window.
///
/// Dimensions are CSS pixels, matching CSSOM View. A viewport is optional
/// because a Window without an associated viewport must report zero for
/// `innerWidth`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportSize {
    /// The viewport width in CSS pixels.
    pub width: u32,
    /// The viewport height in CSS pixels.
    pub height: u32,
}

/// Registers Window globals with a renderer security context and viewport.
pub fn register_window_with_security_context_and_viewport(
    realm: &mut Realm,
    security_context: RendererSecurityContext,
    viewport: ViewportSize,
) -> JsResult<()> {
    register_window_with_origin(realm, security_context.origin().clone(), Some(viewport))
}

fn register_window_with_origin(
    realm: &mut Realm,
    origin: Origin,
    viewport: Option<ViewportSize>,
) -> JsResult<()> {
    let window = match realm.registered_global_value("window") {
        Some(Value::Object(window)) => window,
        _ => Object::new(),
    };
    let custom_elements_object = match realm.registered_global_value("customElements") {
        Some(Value::Object(registry)) => registry,
        _ => CustomElementRegistry::new().object(),
    };
    window.set(
        "customElements",
        Value::Object(custom_elements_object.clone()),
    );
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
    window.set(
        "innerWidth",
        Value::Number(viewport.map_or(0, |size| size.width) as f64),
    );

    let window_value = Value::Object(window);
    realm.register_global_value("customElements", Value::Object(custom_elements_object))?;
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
    fn inner_width_reports_the_viewport_width_in_css_pixels() {
        let mut realm = Realm::new();
        super::register_window_with_security_context_and_viewport(
            &mut realm,
            RendererSecurityContext::new(
                Origin::new("https", "example.com", None),
                StoragePermission::Allowed,
            ),
            super::ViewportSize {
                width: 800,
                height: 600,
            },
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script("window.innerWidth"),
            Ok(Value::Number(800.0))
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
}
