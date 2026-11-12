//! JavaScript `localStorage` bindings.

use std::sync::{Arc, Mutex, OnceLock};

use common::ipc::StorageRequest;
use js::{HostFunction, JsError, JsResult, Object, Realm, Value, WeakObject};
use security::{RendererSecurityContext, StorageAuthorizationError};
use storage::{SharedStorage, StorageError};

/// JavaScript-facing storage for one renderer context.
#[derive(Debug, Clone)]
pub struct LocalStorage {
    storage: SharedStorage,
    security_context: RendererSecurityContext,
    window: Option<Object>,
    document_url: String,
}

#[derive(Clone)]
struct StorageSubscriber {
    origin: String,
    window: WeakObject,
    storage_area: WeakObject,
}

fn storage_subscribers() -> &'static Mutex<Vec<StorageSubscriber>> {
    static SUBSCRIBERS: OnceLock<Mutex<Vec<StorageSubscriber>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(|| Mutex::new(Vec::new()))
}

fn dom_exception(name: &str, message: impl Into<String>) -> JsError {
    let exception = Object::new();
    exception.set("name", Value::String(name.to_owned()));
    exception.set("message", Value::String(message.into()));
    exception.set("code", Value::Number(0.0));
    JsError::thrown(Value::Object(exception))
}

fn storage_error(error: StorageError) -> JsError {
    match error {
        error @ StorageError::QuotaExceeded { .. } => {
            dom_exception("QuotaExceededError", error.to_string())
        }
        other => JsError::new(other.to_string()),
    }
}

impl LocalStorage {
    /// Creates a binding for one trusted renderer context.
    #[must_use]
    pub fn new(storage: SharedStorage, security_context: RendererSecurityContext) -> Self {
        let origin = security_context.origin();
        Self {
            storage,
            document_url: format!("{}://{}:{}", origin.scheme(), origin.host(), origin.port()),
            security_context,
            window: None,
        }
    }

    fn with_url(
        storage: SharedStorage,
        security_context: RendererSecurityContext,
        document_url: impl Into<String>,
    ) -> Self {
        Self {
            storage,
            security_context,
            document_url: document_url.into(),
            window: None,
        }
    }

    fn attach_window(&mut self, window: Object) {
        self.window = Some(window);
    }

    /// Stores a value in this binding's origin area.
    pub fn set_item(&self, key: &str, value: &str) -> JsResult<()> {
        let origin = self.security_context.origin();
        let request = StorageRequest::SetItem {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        let authorized = self.authorize_request(&request, "write")?;

        log::debug!(
            "localStorage.setItem key {key:?} ({} bytes) for {}://{}:{}",
            value.len(),
            origin.scheme(),
            origin.host(),
            origin.port()
        );
        let old_value = self.get_item(key)?;
        self.storage
            .lock()
            .map_err(|_| {
                log::error!("localStorage backend lock is poisoned");
                JsError::new("localStorage is unavailable")
            })?
            .write_item(&authorized.storage_key(), key, value)
            .map_err(|error| {
                log::debug!("localStorage.setItem key {key:?} failed: {error}");
                storage_error(error)
            })?;
        if old_value.as_deref() != Some(value) {
            self.notify(key, old_value, Some(value.to_owned()));
        }
        Ok(())
    }

    /// Reads a value from this binding's origin area.
    pub fn get_item(&self, key: &str) -> JsResult<Option<String>> {
        let origin = self.security_context.origin();
        let request = StorageRequest::GetItem {
            key: key.to_owned(),
        };
        let authorized = self.authorize_request(&request, "read")?;

        let value = self
            .storage
            .lock()
            .map_err(|_| {
                log::error!("localStorage backend lock is poisoned");
                JsError::new("localStorage is unavailable")
            })?
            .read_item(&authorized.storage_key(), key)
            .map_err(|error| {
                log::debug!("localStorage.getItem key {key:?} failed: {error}");
                storage_error(error)
            })?;
        log::debug!(
            "localStorage.getItem key {key:?} for {}://{}:{} (found: {}, {} bytes)",
            origin.scheme(),
            origin.host(),
            origin.port(),
            value.is_some(),
            value.as_ref().map_or(0, String::len)
        );
        Ok(value)
    }

    /// Returns the stable, sorted key list for this origin area.
    pub fn keys(&self) -> JsResult<Vec<String>> {
        let authorized = self.authorize_request(&StorageRequest::Length, "read")?;
        let mut keys = self
            .storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .keys(&authorized.storage_key())
            .map_err(storage_error)?;
        keys.sort();
        Ok(keys)
    }

    /// Removes a key from this origin area.
    pub fn remove_item(&self, key: &str) -> JsResult<()> {
        let request = StorageRequest::RemoveItem {
            key: key.to_owned(),
        };
        let authorized = self.authorize_request(&request, "write")?;
        let old_value = self.get_item(key)?;
        self.storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .remove_item(&authorized.storage_key(), key)
            .map_err(storage_error)?;
        if old_value.is_some() {
            self.notify(key, old_value, None);
        }
        Ok(())
    }

    /// Clears this origin area.
    pub fn clear(&self) -> JsResult<()> {
        let authorized = self.authorize_request(&StorageRequest::Clear, "write")?;
        let had_items = self
            .storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .keys(&authorized.storage_key())
            .map_err(|error| JsError::new(error.to_string()))?
            .into_iter()
            .next()
            .is_some();
        self.storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .clear(&authorized.storage_key())
            .map_err(storage_error)?;
        if had_items {
            self.notify("", None, None);
        }
        Ok(())
    }

    fn notify(&self, key: &str, old_value: Option<String>, new_value: Option<String>) {
        let Some(source_window) = self.window.as_ref() else {
            return;
        };
        let origin = self.security_context.origin();
        let origin = format!("{}://{}:{}", origin.scheme(), origin.host(), origin.port());
        let mut subscribers = storage_subscribers()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        subscribers.retain(|subscriber| subscriber.window.upgrade().is_some());
        for subscriber in subscribers.iter() {
            let Some(window) = subscriber.window.upgrade() else {
                continue;
            };
            if subscriber.origin != origin || window == *source_window {
                continue;
            }
            let Some(storage_area) = subscriber.storage_area.upgrade() else {
                continue;
            };
            let event = Object::new();
            event.set("type", Value::String("storage".into()));
            event.set("bubbles", Value::Boolean(false));
            event.set("cancelable", Value::Boolean(false));
            event.set("key", Value::String(key.to_owned()));
            event.set(
                "oldValue",
                old_value.clone().map_or(Value::Null, Value::String),
            );
            event.set(
                "newValue",
                new_value.clone().map_or(Value::Null, Value::String),
            );
            event.set("url", Value::String(self.document_url.clone()));
            event.set("storageArea", Value::Object(storage_area));
            if let Some(Value::Object(listeners)) = window.get("__storageListeners") {
                for listener_name in listeners.keys() {
                    if let Some(Value::Function(listener)) = listeners.get(&listener_name)
                        && let Err(error) = listener.call(&[Value::Object(event.clone())])
                    {
                        log::warn!("storage event listener threw: {error}");
                    }
                }
            }
        }
    }

    fn authorize_request(
        &self,
        request: &StorageRequest,
        operation: &str,
    ) -> JsResult<security::AuthorizedStorageRequest<'_>> {
        self.security_context
            .authorize_storage_request(request, self.security_context.storage_key())
            .map_err(|error| {
                log::debug!("localStorage {operation} denied: {error}");
                match error {
                    StorageAuthorizationError::PermissionDenied
                    | StorageAuthorizationError::StorageKeyMismatch => dom_exception(
                        "SecurityError",
                        format!("localStorage {operation} access denied"),
                    ),
                    StorageAuthorizationError::InvalidRequest => {
                        dom_exception("SecurityError", "localStorage request is invalid")
                    }
                }
            })
    }
}

/// Implements JavaScript `localStorage.setItem(key, value)`.
pub fn set_item(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let key = arguments.first().unwrap_or(&Value::Undefined).to_string();
    let value = arguments.get(1).unwrap_or(&Value::Undefined).to_string();
    storage.set_item(&key, &value)?;
    Ok(Value::Undefined)
}

/// Implements JavaScript `localStorage.getItem(key)`.
pub fn get_item(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let key = arguments.first().unwrap_or(&Value::Undefined).to_string();
    Ok(match storage.get_item(&key)? {
        Some(value) => Value::String(value),
        None => Value::Null,
    })
}

fn remove_item(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let key = arguments.first().unwrap_or(&Value::Undefined).to_string();
    storage.remove_item(&key)?;
    Ok(Value::Undefined)
}

fn clear(storage: &LocalStorage, _arguments: &[Value]) -> JsResult<Value> {
    storage.clear()?;
    Ok(Value::Undefined)
}

fn length(storage: &LocalStorage, _arguments: &[Value]) -> JsResult<Value> {
    Ok(Value::Number(storage.keys()?.len() as f64))
}

fn key(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let index = arguments
        .first()
        .unwrap_or(&Value::Undefined)
        .to_string()
        .parse::<usize>()
        .unwrap_or(0);
    Ok(storage
        .keys()?
        .get(index)
        .cloned()
        .map_or(Value::Null, Value::String))
}

/// Registers origin-scoped `localStorage` functions in a JavaScript realm.
pub fn register_local_storage(
    realm: &mut Realm,
    storage: SharedStorage,
    security_context: RendererSecurityContext,
) -> JsResult<()> {
    let document_url = {
        let origin = security_context.origin();
        format!("{}://{}:{}", origin.scheme(), origin.host(), origin.port())
    };
    register_local_storage_with_url(realm, storage, security_context, document_url)
}

/// Registers origin-scoped `localStorage` using the document URL for events.
pub fn register_local_storage_with_url(
    realm: &mut Realm,
    storage: SharedStorage,
    security_context: RendererSecurityContext,
    document_url: impl Into<String>,
) -> JsResult<()> {
    log::debug!("registering localStorage bindings");
    let mut local_storage = LocalStorage::with_url(storage, security_context, document_url);
    let window = realm
        .registered_global_value("window")
        .and_then(|value| match value {
            Value::Object(window) => Some(window),
            _ => None,
        });
    if let Some(window) = window.as_ref() {
        local_storage.attach_window(window.clone());
    }
    let object = Object::new();
    let set_storage = local_storage.clone();
    object.set_method(
        "setItem",
        Arc::new(move |arguments: &[Value]| set_item(&set_storage, arguments)) as HostFunction,
    );
    let get_storage = local_storage.clone();
    object.set_method(
        "getItem",
        Arc::new(move |arguments: &[Value]| get_item(&get_storage, arguments)) as HostFunction,
    );
    let remove_storage = local_storage.clone();
    object.set_method(
        "removeItem",
        Arc::new(move |arguments: &[Value]| remove_item(&remove_storage, arguments))
            as HostFunction,
    );
    let clear_storage = local_storage.clone();
    object.set_method(
        "clear",
        Arc::new(move |arguments: &[Value]| clear(&clear_storage, arguments)) as HostFunction,
    );
    let length_storage = local_storage.clone();
    object.define_accessor(
        "length",
        Arc::new(move || length(&length_storage, &[])),
        None,
    );
    let set_compat_storage = local_storage.clone();
    let get_compat_storage = local_storage.clone();
    let key_storage = local_storage.clone();
    object.set_method(
        "key",
        Arc::new(move |arguments: &[Value]| key(&key_storage, arguments)) as HostFunction,
    );
    let property_get_storage = local_storage.clone();
    let property_set_storage = property_get_storage.clone();
    let property_keys_storage = property_get_storage.clone();
    object.set_property_hooks(
        Arc::new(move |property| {
            property_get_storage
                .get_item(property)
                .map(|value| value.map(Value::String))
        }),
        Arc::new(move |property, value| {
            property_set_storage.set_item(property, &value.to_string())?;
            Ok(true)
        }),
        Arc::new(move || property_keys_storage.keys()),
    );
    realm.register_global_value("localStorage", Value::Object(object.clone()))?;
    realm.register_global_function(
        "localStorage.setItem",
        Arc::new(move |arguments: &[Value]| set_item(&set_compat_storage, arguments))
            as HostFunction,
    )?;
    realm.register_global_function(
        "localStorage.getItem",
        Arc::new(move |arguments: &[Value]| get_item(&get_compat_storage, arguments))
            as HostFunction,
    )?;
    if let Some(window) = window {
        window.set("localStorage", Value::Object(object));
        let origin = local_storage.security_context.origin();
        storage_subscribers()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(StorageSubscriber {
                origin: format!("{}://{}:{}", origin.scheme(), origin.host(), origin.port()),
                window: window.downgrade(),
                storage_area: match realm.registered_global_value("localStorage") {
                    Some(Value::Object(storage_area)) => storage_area.downgrade(),
                    _ => return Err(JsError::new("localStorage registration failed")),
                },
            });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use js::{Realm, Value};
    use security::{Origin, RendererSecurityContext, StoragePermission};
    use storage::{
        OriginStorage, PersistentOriginStorage, SharedStorage, StorageBackend, StorageError,
        shared_storage,
    };

    use super::{LocalStorage, get_item, register_local_storage, set_item};
    use crate::register_window;

    fn shared_memory_storage() -> SharedStorage {
        shared_storage(OriginStorage::new())
    }

    fn context(host: &str, permission: StoragePermission) -> RendererSecurityContext {
        RendererSecurityContext::new(Origin::with_port("https", host, 443), permission)
    }

    fn test_directory() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("byui-webapis-storage-{suffix}"))
    }

    #[test]
    fn set_then_get_round_trips_through_binding() {
        let binding = LocalStorage::new(
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        );

        binding.set_item("name", "Nestor").unwrap();

        assert_eq!(binding.get_item("name").unwrap(), Some("Nestor".into()));
    }

    #[test]
    fn missing_items_return_none() {
        let binding = LocalStorage::new(
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        );

        assert_eq!(binding.get_item("missing").unwrap(), None);
    }

    #[test]
    fn javascript_set_item_writes_to_storage() {
        let binding = LocalStorage::new(
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        );

        let result = set_item(
            &binding,
            &[
                Value::String("name".to_owned()),
                Value::String("Nestor".to_owned()),
            ],
        );

        assert_eq!(result, Ok(Value::Undefined));
        assert_eq!(binding.get_item("name").unwrap(), Some("Nestor".into()));
    }

    #[test]
    fn javascript_get_item_returns_null_for_missing_items() {
        let binding = LocalStorage::new(
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        );

        assert_eq!(
            get_item(&binding, &[Value::String("name".to_owned())]),
            Ok(Value::Null)
        );
    }

    #[test]
    fn registered_functions_use_real_storage() {
        let mut realm = Realm::new();
        register_local_storage(
            &mut realm,
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        )
        .unwrap();

        assert_eq!(
            realm.call_global(
                "localStorage.setItem",
                &[
                    Value::String("name".to_owned()),
                    Value::String("Nestor".to_owned())
                ],
            ),
            Ok(Value::Undefined)
        );
        assert_eq!(
            realm.call_global("localStorage.getItem", &[Value::String("name".to_owned())],),
            Ok(Value::String("Nestor".to_owned()))
        );
    }

    #[test]
    fn storage_object_follows_web_storage_contract() {
        let mut realm = Realm::new();
        register_local_storage(
            &mut realm,
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        )
        .unwrap();

        assert_eq!(
            realm.evaluate_script(
                "localStorage.clear(); localStorage.setItem('answer', 42); localStorage.length === 1 && localStorage.key(0) === 'answer' && localStorage.getItem('answer') === '42'",
            ),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script(
                "localStorage.removeItem('answer'); localStorage.getItem('answer') === null && localStorage.length === 0",
            ),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            realm.evaluate_script("localStorage.answer = 7; localStorage.answer === '7'"),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn storage_events_reach_other_windows_of_the_same_origin() {
        let storage = shared_memory_storage();
        let mut first = Realm::new();
        let mut second = Realm::new();
        register_window(&mut first).unwrap();
        register_window(&mut second).unwrap();
        register_local_storage(
            &mut first,
            storage.clone(),
            context("example.com", StoragePermission::Allowed),
        )
        .unwrap();
        register_local_storage(
            &mut second,
            storage,
            context("example.com", StoragePermission::Allowed),
        )
        .unwrap();

        second
            .evaluate_script(
                "window.addEventListener('storage', (event) => window.seen = event.key + ':' + event.newValue)",
            )
            .unwrap();
        first
            .evaluate_script("localStorage.setItem('theme', 'dark')")
            .unwrap();

        assert_eq!(
            second.evaluate_script("window.seen === 'theme:dark'"),
            Ok(Value::Boolean(true))
        );
        assert_eq!(
            first.evaluate_script("window.seen !== 'theme:dark'"),
            Ok(Value::Boolean(true))
        );
    }

    #[test]
    fn registered_functions_keep_origins_isolated() {
        let storage = shared_memory_storage();
        let mut first_realm = Realm::new();
        let mut second_realm = Realm::new();
        register_local_storage(
            &mut first_realm,
            storage.clone(),
            context("first.example", StoragePermission::Allowed),
        )
        .unwrap();
        register_local_storage(
            &mut second_realm,
            storage,
            context("second.example", StoragePermission::Allowed),
        )
        .unwrap();

        first_realm
            .call_global(
                "localStorage.setItem",
                &[
                    Value::String("token".to_owned()),
                    Value::String("first".to_owned()),
                ],
            )
            .unwrap();

        assert_eq!(
            second_realm.call_global("localStorage.getItem", &[Value::String("token".to_owned())],),
            Ok(Value::Null)
        );
    }

    #[test]
    fn javascript_arguments_are_validated() {
        let binding = LocalStorage::new(
            shared_memory_storage(),
            context("example.com", StoragePermission::Allowed),
        );

        assert_eq!(
            set_item(&binding, &[Value::String("only-key".to_owned())]),
            Ok(Value::Undefined)
        );
        assert_eq!(get_item(&binding, &[]), Ok(Value::Null));
    }

    #[test]
    fn denied_context_cannot_read_or_write_storage() {
        let binding = LocalStorage::new(
            shared_memory_storage(),
            context("example.com", StoragePermission::Denied),
        );

        assert!(binding.set_item("name", "Nestor").is_err());
        assert!(binding.get_item("name").is_err());
    }

    #[derive(Debug)]
    struct PanicBackend;

    impl StorageBackend for PanicBackend {
        fn read_item(
            &mut self,
            _storage_key: &security::TrustedStorageKey<'_>,
            _key: &str,
        ) -> Result<Option<String>, StorageError> {
            panic!("denied request reached storage backend");
        }

        fn write_item(
            &mut self,
            _storage_key: &security::TrustedStorageKey<'_>,
            _key: &str,
            _value: &str,
        ) -> Result<(), StorageError> {
            panic!("denied request reached storage backend");
        }

        fn keys(
            &mut self,
            _storage_key: &security::TrustedStorageKey<'_>,
        ) -> Result<Vec<String>, StorageError> {
            panic!("denied request reached storage backend");
        }

        fn remove_item(
            &mut self,
            _storage_key: &security::TrustedStorageKey<'_>,
            _key: &str,
        ) -> Result<(), StorageError> {
            panic!("denied request reached storage backend");
        }

        fn clear(
            &mut self,
            _storage_key: &security::TrustedStorageKey<'_>,
        ) -> Result<(), StorageError> {
            panic!("denied request reached storage backend");
        }
    }

    #[test]
    fn denied_requests_are_rejected_before_backend_access() {
        let binding = LocalStorage::new(
            shared_storage(PanicBackend),
            context("example.com", StoragePermission::Denied),
        );

        assert!(binding.get_item("name").is_err());
        assert!(binding.set_item("name", "Nestor").is_err());
    }

    #[test]
    fn javascript_binding_survives_backend_reopen() {
        let path = test_directory();
        let renderer_context = context("example.com", StoragePermission::Allowed);

        {
            let mut realm = Realm::new();
            register_local_storage(
                &mut realm,
                shared_storage(PersistentOriginStorage::open(&path).unwrap()),
                renderer_context.clone(),
            )
            .unwrap();
            realm
                .call_global(
                    "localStorage.setItem",
                    &[
                        Value::String("theme".to_owned()),
                        Value::String("dark".to_owned()),
                    ],
                )
                .unwrap();
        }

        let mut reopened_realm = Realm::new();
        register_local_storage(
            &mut reopened_realm,
            shared_storage(PersistentOriginStorage::open(&path).unwrap()),
            renderer_context,
        )
        .unwrap();
        assert_eq!(
            reopened_realm
                .call_global("localStorage.getItem", &[Value::String("theme".to_owned())],),
            Ok(Value::String("dark".to_owned()))
        );

        let _ = std::fs::remove_dir_all(path);
    }
}
