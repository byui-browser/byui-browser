//! JavaScript `localStorage` bindings.

use std::sync::Arc;

use js::{HostFunction, JsError, JsResult, Realm, Value};
use security::RendererSecurityContext;
use storage::SharedStorage;

/// JavaScript-facing storage for one renderer context.
#[derive(Debug, Clone)]
pub struct LocalStorage {
    storage: SharedStorage,
    security_context: RendererSecurityContext,
}

impl LocalStorage {
    /// Creates a binding for one trusted renderer context.
    #[must_use]
    pub fn new(storage: SharedStorage, security_context: RendererSecurityContext) -> Self {
        Self {
            storage,
            security_context,
        }
    }

    /// Stores a value in this binding's origin area.
    pub fn set_item(&self, key: &str, value: &str) -> JsResult<()> {
        if !self.security_context.can_write_storage() {
            return Err(JsError::new("localStorage write access denied"));
        }

        self.storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .write_item(self.security_context.storage_key(), key, value)
            .map_err(|error| JsError::new(error.to_string()))
    }

    /// Reads a value from this binding's origin area.
    pub fn get_item(&self, key: &str) -> JsResult<Option<String>> {
        if !self.security_context.can_read_storage() {
            return Err(JsError::new("localStorage read access denied"));
        }

        self.storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .read_item(self.security_context.storage_key(), key)
            .map_err(|error| JsError::new(error.to_string()))
    }
}

/// Implements JavaScript `localStorage.setItem(key, value)`.
pub fn set_item(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(key), Value::String(value)] = arguments else {
        return Err(JsError::new(
            "localStorage.setItem() expects a key and value string",
        ));
    };

    storage.set_item(key, value)?;
    Ok(Value::Undefined)
}

/// Implements JavaScript `localStorage.getItem(key)`.
pub fn get_item(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(key)] = arguments else {
        return Err(JsError::new(
            "localStorage.getItem() expects one key string",
        ));
    };

    Ok(match storage.get_item(key)? {
        Some(value) => Value::String(value),
        None => Value::Null,
    })
}

/// Registers origin-scoped `localStorage` functions in a JavaScript realm.
pub fn register_local_storage(
    realm: &mut Realm,
    storage: SharedStorage,
    security_context: RendererSecurityContext,
) -> JsResult<()> {
    let local_storage = LocalStorage::new(storage, security_context);
    let set_item_storage = local_storage.clone();
    let get_item_storage = local_storage;
    let set_item_function: HostFunction =
        Arc::new(move |arguments| set_item(&set_item_storage, arguments));
    let get_item_function: HostFunction =
        Arc::new(move |arguments| get_item(&get_item_storage, arguments));

    realm.register_global_function("localStorage.setItem", set_item_function)?;
    realm.register_global_function("localStorage.getItem", get_item_function)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use js::{Realm, Value};
    use security::{Origin, RendererSecurityContext, StoragePermission};
    use storage::{OriginStorage, PersistentOriginStorage, SharedStorage, shared_storage};

    use super::{LocalStorage, get_item, register_local_storage, set_item};

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
    fn denied_context_cannot_read_or_write_storage() {
        let denied_context = RendererSecurityContext::new(
            Origin::with_port("https", "example.com", 443),
            StoragePermission::Denied,
        );
        let binding = LocalStorage::new(shared_storage(), denied_context);

        assert_eq!(
            binding.set_item("name", "Nestor").unwrap_err().to_string(),
            "storage write denied"
        );
        assert_eq!(
            binding.get_item("name").unwrap_err().to_string(),
            "storage read denied"
        );
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

        assert!(set_item(&binding, &[Value::String("only-key".to_owned())]).is_err());
        assert!(get_item(&binding, &[]).is_err());
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
