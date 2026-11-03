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
