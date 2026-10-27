//! Public-contract tests for `storage`.
//!
//! Run ignored tests with `cargo test -p storage -- --ignored` to see the backlog.

use security::{Origin, StorageKey};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use storage::{LocalStorage, OriginStorage, PersistentOriginStorage};

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

fn test_directory(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("byui-storage-contract-{name}-{suffix}"))
}

#[test]
fn values_survive_reopen() {
    let path = test_directory("reopen");
    let key = StorageKey::new(Origin::with_port("https", "example.com", 443));

    {
        let mut store = PersistentOriginStorage::open(&path).unwrap();
        store.write_item(&key, "token", "abc").unwrap();
    }

    let mut reopened = PersistentOriginStorage::open(&path).unwrap();
    assert_eq!(
        reopened.read_item(&key, "token").unwrap(),
        Some("abc".into())
    );
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn persistent_storage_is_origin_scoped() {
    let path = test_directory("isolation");
    let first = StorageKey::new(Origin::with_port("https", "example.com", 443));
    let second = StorageKey::new(Origin::with_port("https", "other.example", 443));
    let mut store = PersistentOriginStorage::open(&path).unwrap();

    store.write_item(&first, "token", "first").unwrap();
    store.write_item(&second, "token", "second").unwrap();
    drop(store);

    let mut reopened = PersistentOriginStorage::open(&path).unwrap();
    assert_eq!(
        reopened.read_item(&first, "token").unwrap(),
        Some("first".into())
    );
    assert_eq!(
        reopened.read_item(&second, "token").unwrap(),
        Some("second".into())
    );
    let _ = std::fs::remove_dir_all(path);
}
