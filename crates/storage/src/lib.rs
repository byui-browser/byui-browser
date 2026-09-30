//! Client-side storage: IndexedDB, localStorage, Cache API, and related persistence.
//!
//! **Owning team**: Security & Storage Team
//!
//! Runs primarily in the Utility / Storage process.

#![forbid(unsafe_code)]

use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
