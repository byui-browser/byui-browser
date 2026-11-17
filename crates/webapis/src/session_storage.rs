//! JavaScript `sessionStorage` bindings.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::ipc::StorageRequest;
use js::{HostFunction, JsError, JsResult, Object, Realm, Value};
use security::{RendererSecurityContext, StorageAuthorizationError};

#[derive(Debug, Clone)]
struct SessionStorage {
    items: Arc<Mutex<HashMap<String, String>>>,
    security_context: RendererSecurityContext,
}

impl SessionStorage {
    fn new(security_context: RendererSecurityContext) -> Self {
        Self {
            items: Arc::new(Mutex::new(HashMap::new())),
            security_context,
        }
    }

    fn authorize(&self, request: &StorageRequest, operation: &str) -> JsResult<()> {
        self.security_context
            .authorize_storage_request(request, self.security_context.storage_key())
            .map(|_| ())
            .map_err(|error| match error {
                StorageAuthorizationError::PermissionDenied
                | StorageAuthorizationError::StorageKeyMismatch => {
                    JsError::new(format!("sessionStorage {operation} access denied"))
                }
                StorageAuthorizationError::InvalidRequest => {
                    JsError::new("sessionStorage request is invalid")
                }
            })
    }

    fn with_items<T>(
        &self,
        operation: &str,
        action: impl FnOnce(&mut HashMap<String, String>) -> T,
    ) -> JsResult<T> {
        self.items
            .lock()
            .map(|mut items| action(&mut items))
            .map_err(|_| JsError::new(format!("sessionStorage is unavailable during {operation}")))
    }
}

fn string_argument(arguments: &[Value], index: usize) -> String {
    arguments
        .get(index)
        .unwrap_or(&Value::Undefined)
        .to_string()
}

fn set_item(storage: &SessionStorage, arguments: &[Value]) -> JsResult<Value> {
    let key = string_argument(arguments, 0);
    let value = string_argument(arguments, 1);
    storage.authorize(
        &StorageRequest::SetItem {
            key: key.clone(),
            value: value.clone(),
        },
        "write",
    )?;
    storage.with_items("setItem", |items| {
        items.insert(key, value);
    })?;
    Ok(Value::Undefined)
}

fn get_item(storage: &SessionStorage, arguments: &[Value]) -> JsResult<Value> {
    let key = string_argument(arguments, 0);
    storage.authorize(&StorageRequest::GetItem { key: key.clone() }, "read")?;
    Ok(storage
        .with_items("getItem", |items| items.get(&key).cloned())?
        .map_or(Value::Null, Value::String))
}

fn remove_item(storage: &SessionStorage, arguments: &[Value]) -> JsResult<Value> {
    let key = string_argument(arguments, 0);
    storage.authorize(&StorageRequest::RemoveItem { key: key.clone() }, "write")?;
    storage.with_items("removeItem", |items| {
        items.remove(&key);
    })?;
    Ok(Value::Undefined)
}

fn clear(storage: &SessionStorage, _arguments: &[Value]) -> JsResult<Value> {
    storage.authorize(&StorageRequest::Clear, "write")?;
    storage.with_items("clear", HashMap::clear)?;
    Ok(Value::Undefined)
}

fn length(storage: &SessionStorage, _arguments: &[Value]) -> JsResult<Value> {
    storage.authorize(&StorageRequest::Length, "read")?;
    Ok(Value::Number(
        storage.with_items("length", |items| items.len())? as f64,
    ))
}

fn key(storage: &SessionStorage, arguments: &[Value]) -> JsResult<Value> {
    let index = string_argument(arguments, 0).parse::<usize>().unwrap_or(0);
    storage.authorize(&StorageRequest::Length, "read")?;
    let value = storage.with_items("key", |items| items.keys().nth(index).cloned())?;
    Ok(value.map_or(Value::Null, Value::String))
}

/// Registers a fresh, page-scoped `window.sessionStorage` object.
///
/// The current JavaScript object model does not yet provide dynamic named
/// properties or cross-window event delivery, so this binding exposes the
/// standard methods and `length` accessor. The storage area itself remains
/// isolated to the registered page realm and is never persisted.
pub fn register_session_storage(
    realm: &mut Realm,
    security_context: RendererSecurityContext,
) -> JsResult<()> {
    let storage = SessionStorage::new(security_context);
    let object = Object::new();
    let set_storage = storage.clone();
    object.set_method(
        "setItem",
        Arc::new(move |arguments: &[Value]| set_item(&set_storage, arguments)) as HostFunction,
    );
    let get_storage = storage.clone();
    object.set_method(
        "getItem",
        Arc::new(move |arguments: &[Value]| get_item(&get_storage, arguments)) as HostFunction,
    );
    let remove_storage = storage.clone();
    object.set_method(
        "removeItem",
        Arc::new(move |arguments: &[Value]| remove_item(&remove_storage, arguments))
            as HostFunction,
    );
    let clear_storage = storage.clone();
    object.set_method(
        "clear",
        Arc::new(move |arguments: &[Value]| clear(&clear_storage, arguments)) as HostFunction,
    );
    let length_storage = storage.clone();
    object.define_accessor(
        "length",
        Arc::new(move || length(&length_storage, &[])),
        None,
    );
    let key_storage = storage;
    object.set_method(
        "key",
        Arc::new(move |arguments: &[Value]| key(&key_storage, arguments)) as HostFunction,
    );

    realm.register_global_value("sessionStorage", Value::Object(object.clone()))?;
    if let Some(Value::Object(window)) = realm.registered_global_value("window") {
        window.set("sessionStorage", Value::Object(object));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use js::Realm;
    use security::{Origin, RendererSecurityContext, StoragePermission};

    use super::register_session_storage;
    use crate::register_window;

    fn context(permission: StoragePermission) -> RendererSecurityContext {
        RendererSecurityContext::new(Origin::with_port("https", "example.com", 443), permission)
    }

    #[test]
    fn storage_methods_follow_the_web_storage_contract() {
        let mut realm = Realm::new();
        register_session_storage(&mut realm, context(StoragePermission::Allowed)).unwrap();

        assert_eq!(
            realm.evaluate_script(
                "sessionStorage.setItem('answer', 42); sessionStorage.length === 1 && sessionStorage.getItem('answer') === '42'"
            ),
            Ok(js::Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script("sessionStorage.key(0) === 'answer'"),
            Ok(js::Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script("sessionStorage.removeItem('answer'); sessionStorage.getItem('answer') === null && sessionStorage.length === 0"),
            Ok(js::Value::Boolean(true))
        );
    }

    #[test]
    fn session_storage_is_exposed_on_window_and_is_page_scoped() {
        let mut first = Realm::new();
        let mut second = Realm::new();
        register_window(&mut first).unwrap();
        register_window(&mut second).unwrap();
        register_session_storage(&mut first, context(StoragePermission::Allowed)).unwrap();
        register_session_storage(&mut second, context(StoragePermission::Allowed)).unwrap();

        first
            .evaluate_script("window.sessionStorage.setItem('token', 'first')")
            .unwrap();
        assert_eq!(
            second.evaluate_script("sessionStorage.getItem('token') === null"),
            Ok(js::Value::Boolean(true))
        );
    }

    #[test]
    fn denied_context_cannot_access_session_storage() {
        let mut realm = Realm::new();
        register_session_storage(&mut realm, context(StoragePermission::Denied)).unwrap();

        assert!(
            realm
                .evaluate_script("sessionStorage.getItem('token')")
                .is_err()
        );
    }
}
