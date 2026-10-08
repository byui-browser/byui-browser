//! Public-contract tests for `storage`.
//!
//! Run ignored tests with `cargo test -p storage -- --ignored` to see the backlog.

use security::{Origin, StorageKey};
use storage::{LocalStorage, OriginStorage};

#[test]
fn public_api_round_trip() {
    let mut store = LocalStorage::new();
    store.set_item("theme", "dark");
    assert_eq!(store.get_item("theme"), Some("dark"));
}

#[test]
fn public_api_is_origin_scoped() {
    let first = StorageKey::new(Origin::with_port("https", "example.com", 443));
    let second = StorageKey::new(Origin::with_port("https", "other.example", 443));
    let mut storage = OriginStorage::new();

    storage.set_item(&first, "theme", "dark");

    assert_eq!(storage.get_item(&first, "theme"), Some("dark"));
    assert_eq!(storage.get_item(&second, "theme"), None);
}

#[test]
#[ignore = "TODO(storage): persistence not implemented; needs an on-disk backend and a reopen API"]
fn values_survive_reopen() {
    // Sketch of the expected contract: write, drop, reopen, read.
    // The team decides the constructor shape (path, origin, or both).
    let mut store = LocalStorage::new();
    store.set_item("token", "abc");
    let reopened = LocalStorage::new();
    assert_eq!(reopened.get_item("token"), Some("abc"));
}
