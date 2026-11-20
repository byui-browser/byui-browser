//! Public-contract tests for `storage`.
//!
//! Run ignored tests with `cargo test -p storage -- --ignored` to see the backlog.

use storage::{LocalStorage, OriginStorage};

mod support;

use support::{other_origin, persistent_storage, primary_origin, storage_key_for, test_directory};

#[test]
fn public_api_round_trip() {
    let mut store = LocalStorage::new();
    store.set_item("theme", "dark");
    assert_eq!(store.get_item("theme"), Some("dark"));
}

#[test]
fn public_api_is_origin_scoped() {
    let first = storage_key_for(primary_origin());
    let second = storage_key_for(other_origin());
    let mut storage = OriginStorage::new();

    storage.set_item(&first, "theme", "dark");

    assert_eq!(storage.get_item(&first, "theme"), Some("dark"));
    assert_eq!(storage.get_item(&second, "theme"), None);
}

#[test]
fn values_survive_reopen() {
    let path = test_directory("reopen");
    let key = storage_key_for(primary_origin());

    {
        let mut store = persistent_storage(&path);
        store.write_item(&key, "token", "abc").unwrap();
    }

    let mut reopened = persistent_storage(&path);
    assert_eq!(
        reopened.read_item(&key, "token").unwrap(),
        Some("abc".into())
    );
}

#[test]
fn persistent_storage_is_origin_scoped() {
    let path = test_directory("isolation");
    let first = storage_key_for(primary_origin());
    let second = storage_key_for(other_origin());
    let mut store = persistent_storage(&path);

    store.write_item(&first, "token", "first").unwrap();
    store.write_item(&second, "token", "second").unwrap();
    drop(store);

    let mut reopened = persistent_storage(&path);
    assert_eq!(
        reopened.read_item(&first, "token").unwrap(),
        Some("first".into())
    );
    assert_eq!(
        reopened.read_item(&second, "token").unwrap(),
        Some("second".into())
    );
}
