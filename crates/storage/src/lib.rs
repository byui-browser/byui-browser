//! Client-side storage: IndexedDB, localStorage, Cache API, and related persistence.
//!
//! **Owning team**: Security & Storage Team
//!
//! Runs primarily in the Utility / Storage process.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use security::StorageKey;

/// In-memory `localStorage` for a single origin.
///
/// The real implementation must be scoped per origin and persisted to disk;
/// this placeholder only fixes the API shape.
// NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team. Reshape the
// types, names, and module layout however your crate's public API needs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LocalStorage {
    items: HashMap<String, String>,
}

impl LocalStorage {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// `localStorage.getItem(key)`.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn get_item(&self, key: &str) -> Option<&str> {
        self.items.get(key).map(String::as_str)
    }

    /// `localStorage.setItem(key, value)`.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn set_item(&mut self, key: &str, value: &str) {
        self.items.insert(key.to_owned(), value.to_owned());
    }

    /// `localStorage.removeItem(key)`; returns the removed value.
    // NOT AUTHORITATIVE: placeholder from the Scrum of Scrums team.
    pub fn remove_item(&mut self, key: &str) -> Option<String> {
        self.items.remove(key)
    }

    /// `localStorage.length`.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the store holds no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// In-memory local storage areas partitioned by [`StorageKey`].
///
/// This is temporary application-lifetime storage. Dropping the manager drops
/// every origin's data; no values are written to disk. Each origin gets an
/// independent local-storage area, so equal keys on different origins cannot
/// see one another's values.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OriginStorage {
    areas: HashMap<StorageKey, LocalStorage>,
}

impl OriginStorage {
    /// Creates an empty origin-partitioned storage manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the value for `key` in the area identified by `storage_key`.
    ///
    /// Returns `None` when the origin has no area or the key is not present.
    #[must_use]
    pub fn get_item(&self, storage_key: &StorageKey, key: &str) -> Option<&str> {
        self.areas
            .get(storage_key)
            .and_then(|area| area.get_item(key))
    }

    /// Stores `value` under `key` in the area identified by `storage_key`.
    pub fn set_item(&mut self, storage_key: &StorageKey, key: &str, value: &str) {
        self.areas
            .entry(storage_key.clone())
            .or_default()
            .set_item(key, value);
    }

    /// Removes and returns the value for `key` in the selected origin area.
    pub fn remove_item(&mut self, storage_key: &StorageKey, key: &str) -> Option<String> {
        let value = self.areas.get_mut(storage_key)?.remove_item(key);
        if self
            .areas
            .get(storage_key)
            .is_some_and(LocalStorage::is_empty)
        {
            self.areas.remove(storage_key);
        }
        value
    }

    /// Returns the number of stored keys in the selected origin area.
    #[must_use]
    pub fn len(&self, storage_key: &StorageKey) -> usize {
        self.areas.get(storage_key).map_or(0, LocalStorage::len)
    }

    /// Removes every key in the selected origin area.
    pub fn clear(&mut self, storage_key: &StorageKey) {
        self.areas.remove(storage_key);
    }

    /// Returns whether the selected origin area has no stored keys.
    #[must_use]
    pub fn is_empty(&self, storage_key: &StorageKey) -> bool {
        self.len(storage_key) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use security::{Origin, StorageKey};

    #[test]
    fn missing_key_is_none() {
        assert_eq!(LocalStorage::new().get_item("token"), None);
    }

    #[test]
    fn set_then_get_round_trips() {
        let mut store = LocalStorage::new();
        store.set_item("token", "abc");
        assert_eq!(store.get_item("token"), Some("abc"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn set_overwrites_existing_value() {
        let mut store = LocalStorage::new();
        store.set_item("k", "1");
        store.set_item("k", "2");
        assert_eq!(store.get_item("k"), Some("2"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_returns_old_value_and_clears_key() {
        let mut store = LocalStorage::new();
        store.set_item("k", "v");
        assert_eq!(store.remove_item("k"), Some("v".into()));
        assert!(store.is_empty());
        assert_eq!(store.remove_item("k"), None);
    }

    fn storage_key(scheme: &str, host: &str, port: u16) -> StorageKey {
        StorageKey::new(Origin::with_port(scheme, host, port))
    }

    #[test]
    fn origin_areas_do_not_share_values() {
        let first = storage_key("https", "example.com", 443);
        let second = storage_key("https", "other.example", 443);
        let mut storage = OriginStorage::new();

        storage.set_item(&first, "token", "first");
        storage.set_item(&second, "token", "second");

        assert_eq!(storage.get_item(&first, "token"), Some("first"));
        assert_eq!(storage.get_item(&second, "token"), Some("second"));
    }

    #[test]
    fn same_origin_keys_share_an_area() {
        let first = storage_key("https", "example.com", 443);
        let equivalent = storage_key("HTTPS", "EXAMPLE.COM", 443);
        let mut storage = OriginStorage::new();

        storage.set_item(&first, "theme", "dark");

        assert_eq!(storage.get_item(&equivalent, "theme"), Some("dark"));
        assert_eq!(storage.len(&equivalent), 1);
    }

    #[test]
    fn clear_only_removes_the_selected_origin() {
        let first = storage_key("https", "example.com", 443);
        let second = storage_key("http", "example.com", 80);
        let mut storage = OriginStorage::new();

        storage.set_item(&first, "name", "first");
        storage.set_item(&second, "name", "second");
        storage.clear(&first);

        assert!(storage.is_empty(&first));
        assert_eq!(storage.get_item(&second, "name"), Some("second"));
    }

    #[test]
    fn dropping_storage_discards_all_values() {
        let key = storage_key("https", "example.com", 443);
        {
            let mut storage = OriginStorage::new();
            storage.set_item(&key, "session", "temporary");
        }

        let storage = OriginStorage::new();
        assert!(storage.is_empty(&key));
    }
}
