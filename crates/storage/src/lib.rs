//! Client-side storage: IndexedDB, localStorage, Cache API, and related persistence.
//!
//! **Owning team**: Security & Storage Team
//!
//! Runs primarily in the Utility / Storage process.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use security::{RendererSecurityContext, StorageKey};

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

/// In-memory local storage partitioned by the trusted storage key in a
/// renderer's security context.
///
/// Each operation checks the context's storage permission before accessing its
/// origin's store. Callers cannot select a different origin or storage key for
/// an operation.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OriginStorage {
    stores: HashMap<StorageKey, LocalStorage>,
}

impl OriginStorage {
    /// Creates an empty set of origin-scoped stores.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads `key` from the store associated with `context`.
    ///
    /// Returns `Ok(None)` when the key is absent, or `Err` when the context is
    /// not permitted to read storage.
    pub fn get_item<'a>(
        &'a self,
        context: &RendererSecurityContext,
        key: &str,
    ) -> Result<Option<&'a str>, StorageAccessError> {
        if !context.can_read_storage() {
            return Err(StorageAccessError::ReadDenied);
        }

        Ok(self
            .stores
            .get(context.storage_key())
            .and_then(|store| store.get_item(key)))
    }

    /// Writes `value` to `key` in the store associated with `context`.
    ///
    /// Returns `Err` when the context is not permitted to write storage.
    pub fn set_item(
        &mut self,
        context: &RendererSecurityContext,
        key: &str,
        value: &str,
    ) -> Result<(), StorageAccessError> {
        if !context.can_write_storage() {
            return Err(StorageAccessError::WriteDenied);
        }

        self.stores
            .entry(context.storage_key().clone())
            .or_default()
            .set_item(key, value);
        Ok(())
    }
}

/// A storage operation rejected by the renderer's storage permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageAccessError {
    /// The renderer is not permitted to read storage.
    ReadDenied,
    /// The renderer is not permitted to write storage.
    WriteDenied,
}

impl std::fmt::Display for StorageAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadDenied => f.write_str("storage read denied"),
            Self::WriteDenied => f.write_str("storage write denied"),
        }
    }
}

impl std::error::Error for StorageAccessError {}

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
