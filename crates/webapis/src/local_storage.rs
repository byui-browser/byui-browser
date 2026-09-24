//! Temporary no-op `localStorage` binding.

use std::sync::Arc;

use js::{HostFunction, JsResult, Realm, Value};

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalStorage;

impl LocalStorage {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    pub fn set_item(&self, _key: &str, _value: &str) {}

    #[must_use]
    pub const fn get_item(&self, _key: &str) -> Option<&str> {
        None
    }
}

pub fn set_item(_arguments: &[Value]) -> JsResult<Value> {
    Ok(Value::Undefined)
}

pub fn get_item(_arguments: &[Value]) -> JsResult<Value> {
    Ok(Value::Null)
}

pub fn register_local_storage(realm: &mut Realm) -> JsResult<()> {
    let set_item_function: HostFunction = Arc::new(set_item);
    let get_item_function: HostFunction = Arc::new(get_item);

    realm.register_global_function("localStorage.setItem", set_item_function)?;
    realm.register_global_function("localStorage.getItem", get_item_function)
}

#[cfg(test)]
mod tests {
    use super::{get_item, register_local_storage, set_item, LocalStorage};
    use js::{Realm, Value};

    #[test]
    fn set_item_is_safe_to_call_but_does_not_persist() {
        let storage = LocalStorage::new();

        storage.set_item("name", "Nestor");

        assert_eq!(storage.get_item("name"), None);
    }

    #[test]
    fn missing_items_return_none() {
        let storage = LocalStorage::new();

        assert_eq!(storage.get_item("missing"), None);
    }

    #[test]
    fn javascript_set_item_is_a_no_op() {
        let result = set_item(&[
            Value::String("name".to_owned()),
            Value::String("Nestor".to_owned()),
        ]);

        assert_eq!(result, Ok(Value::Undefined));
    }

    #[test]
    fn javascript_get_item_returns_null_until_storage_exists() {
        assert_eq!(
            get_item(&[Value::String("name".to_owned())]),
            Ok(Value::Null)
        );
    }

    #[test]
    fn local_storage_functions_register_with_the_realm() {
        let mut realm = Realm::new();
        register_local_storage(&mut realm).unwrap();

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
            Ok(Value::Null)
        );
    }
}
