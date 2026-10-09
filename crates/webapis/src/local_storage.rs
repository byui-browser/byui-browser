//! JavaScript `localStorage` bindings backed by in-memory origin storage.

use std::sync::{Arc, Mutex};

use js::{HostFunction, JsError, JsResult, Realm, Value};
use security::StorageKey;
use storage::OriginStorage;

/// JavaScript-facing storage for one trusted origin.
///
/// The storage manager is shared so multiple realms can observe the same
/// origin area. The caller must provide a `StorageKey` from the browser's
/// security context; JavaScript cannot choose or change it.
#[derive(Debug, Clone)]
pub struct LocalStorage {
    storage: Arc<Mutex<OriginStorage>>,
    storage_key: StorageKey,
}

impl LocalStorage {
    /// Creates a binding for one trusted origin.
    #[must_use]
    pub fn new(storage: Arc<Mutex<OriginStorage>>, storage_key: StorageKey) -> Self {
        Self {
            storage,
            storage_key,
        }
    }

    /// Stores a value in this binding's origin area.
    pub fn set_item(&self, key: &str, value: &str) -> JsResult<()> {
        self.storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?
            .set_item(&self.storage_key, key, value);
        Ok(())
    }

    /// Reads a value from this binding's origin area.
    pub fn get_item(&self, key: &str) -> JsResult<Option<String>> {
        let storage = self
            .storage
            .lock()
            .map_err(|_| JsError::new("localStorage is unavailable"))?;
        Ok(storage.get_item(&self.storage_key, key).map(str::to_owned))
    }
}

/// Implements JavaScript `localStorage.setItem(key, value)` for one binding.
pub fn set_item(storage: &LocalStorage, arguments: &[Value]) -> JsResult<Value> {
    let [Value::String(key), Value::String(value)] = arguments else {
        return Err(JsError::new(
            "localStorage.setItem() expects a key and value string",
        ));
    };

    storage.set_item(key, value)?;
    Ok(Value::Undefined)
}

/// Implements JavaScript `localStorage.getItem(key)` for one binding.
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
///
/// This connects JavaScript to in-memory storage only. Persistence, IPC, and
/// renderer authentication are intentionally outside this partial binding.
pub fn register_local_storage(
    realm: &mut Realm,
    storage: Arc<Mutex<OriginStorage>>,
    storage_key: StorageKey,
) -> JsResult<()> {
    let local_storage = LocalStorage::new(storage, storage_key);
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
    use std::sync::{Arc, Mutex};

    use js::{Realm, Value};
    use security::{Origin, StorageKey};
    use storage::OriginStorage;

    use super::{LocalStorage, get_item, register_local_storage, set_item};

    fn storage_key(host: &str) -> StorageKey {
        StorageKey::new(Origin::with_port("https", host, 443))
    }

    fn shared_storage() -> Arc<Mutex<OriginStorage>> {
        Arc::new(Mutex::new(OriginStorage::new()))
    }

    #[test]
    fn set_then_get_round_trips_through_binding() {
        let storage = shared_storage();
        let binding = LocalStorage::new(storage, storage_key("example.com"));

        binding.set_item("name", "Nestor").unwrap();

        assert_eq!(binding.get_item("name").unwrap(), Some("Nestor".into()));
    }

    #[test]
    fn missing_items_return_none() {
        let binding = LocalStorage::new(shared_storage(), storage_key("example.com"));

        assert_eq!(binding.get_item("missing").unwrap(), None);
    }

    #[test]
    fn javascript_set_item_writes_to_storage() {
        let binding = LocalStorage::new(shared_storage(), storage_key("example.com"));

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
        let binding = LocalStorage::new(shared_storage(), storage_key("example.com"));

        assert_eq!(
            get_item(&binding, &[Value::String("name".to_owned())]),
            Ok(Value::Null)
        );
    }

    #[test]
    fn registered_functions_use_real_storage() {
        let storage = shared_storage();
        let mut realm = Realm::new();
        register_local_storage(&mut realm, storage, storage_key("example.com")).unwrap();

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
        let storage = shared_storage();
        let mut first_realm = Realm::new();
        let mut second_realm = Realm::new();
        register_local_storage(
            &mut first_realm,
            Arc::clone(&storage),
            storage_key("first.example"),
        )
        .unwrap();
        register_local_storage(&mut second_realm, storage, storage_key("second.example")).unwrap();

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
        let binding = LocalStorage::new(shared_storage(), storage_key("example.com"));

        assert!(set_item(&binding, &[Value::String("only-key".to_owned())]).is_err());
        assert!(get_item(&binding, &[]).is_err());
    }
}
