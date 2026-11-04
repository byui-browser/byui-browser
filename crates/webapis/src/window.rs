//! The script-visible `Window` object and its self-reference.

use std::sync::Arc;

use crate::custom_elements::CustomElementRegistry;
use js::{JsResult, Object, Realm, Value};

/// Registers the script-visible window globals.
///
/// The current implementation exposes the `window` object and the global
/// `self` alias. Their `self()` method returns the same object identity,
/// which is the supported subset of the Window self-reference API. It also
/// exposes the document's `customElements` registry. Custom element
/// construction and lifecycle support cover the current single-document DOM
/// subset; scoped registries and native promises are not implemented yet.
pub fn register_window(realm: &mut Realm) -> JsResult<()> {
    let window = Object::new();
    let custom_elements_object = match realm.registered_global_value("customElements") {
        Some(Value::Object(registry)) => registry,
        _ => CustomElementRegistry::new().object(),
    };
    window.set(
        "customElements",
        Value::Object(custom_elements_object.clone()),
    );
    let self_window = window.clone();
    window.set_method(
        "self",
        Arc::new(move |_| Ok(Value::Object(self_window.clone()))),
    );

    let window_value = Value::Object(window);
    realm.register_global_value("customElements", Value::Object(custom_elements_object))?;
    realm.register_global_value("window", window_value.clone())?;
    realm.register_global_value("self", window_value)
}
