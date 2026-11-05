//! The script-visible `Window` object and its standard object references.

use std::sync::Arc;

use crate::custom_elements::CustomElementRegistry;
use js::{JsResult, Object, Realm, Value};

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
    window.set_method(
        "self",
        Arc::new(move |_| Ok(Value::Object(self_window.clone()))),
    );
    window.set_method(
        "navigator",
        Arc::new(move |_| Ok(Value::Object(navigator_for_window.clone()))),
    );

    let window_value = Value::Object(window);
    realm.register_global_value("customElements", Value::Object(custom_elements_object))?;
    realm.register_global_value("window", window_value.clone())?;
    realm.register_global_value("self", window_value)?;
    realm.register_global_value("navigator", Value::Object(navigator))
}
