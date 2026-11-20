//! Public-contract tests for `storage`.
//!
//! Run ignored tests with `cargo test -p storage -- --ignored` to see the backlog.

use security::{Origin, StorageKey};
use storage::{LocalStorage, OriginStorage};

mod support;

use support::{
    context_for, other_origin, persistent_storage, primary_origin, storage_key_for, test_directory,
};

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
    let first_context = context_for(&first);
    let second_context = context_for(&second);
    let first = first_context.trusted_storage_key();
    let second = second_context.trusted_storage_key();
    let mut storage = OriginStorage::new();

    storage.set_item(&first, "theme", "dark");

    assert_eq!(storage.get_item(&first, "theme"), Some("dark"));
    assert_eq!(storage.get_item(&second, "theme"), None);
}

#[test]
fn values_survive_reopen() {
    let path = test_directory("reopen");
    let key = storage_key_for(primary_origin());
    let context = context_for(&key);
    let key = context.trusted_storage_key();

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
    let first_context = context_for(&first);
    let second_context = context_for(&second);
    let first = first_context.trusted_storage_key();
    let second = second_context.trusted_storage_key();
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

#[test]
fn different_schemes_cannot_share_storage() {
    let https = StorageKey::new(Origin::with_port("https", "example.com", 443));
    let http = StorageKey::new(Origin::with_port("http", "example.com", 80));
    let https_context = context_for(&https);
    let http_context = context_for(&http);
    let https = https_context.trusted_storage_key();
    let http = http_context.trusted_storage_key();
    let mut storage = OriginStorage::new();

    storage.set_item(&https, "theme", "dark");

    assert_eq!(storage.get_item(&https, "theme"), Some("dark"));
    assert_eq!(storage.get_item(&http, "theme"), None);
}

#[test]
fn different_ports_cannot_share_storage() {
    let standard_port = StorageKey::new(Origin::with_port("https", "example.com", 443));
    let alternate_port = StorageKey::new(Origin::with_port("https", "example.com", 8443));
    let standard_context = context_for(&standard_port);
    let alternate_context = context_for(&alternate_port);
    let standard_port = standard_context.trusted_storage_key();
    let alternate_port = alternate_context.trusted_storage_key();
    let mut storage = OriginStorage::new();

    storage.set_item(&standard_port, "theme", "dark");

    assert_eq!(storage.get_item(&standard_port, "theme"), Some("dark"));
    assert_eq!(storage.get_item(&alternate_port, "theme"), None);
}

#[test]
fn same_origin_pages_share_storage() {
    let first = StorageKey::new(Origin::with_port("https", "example.com", 443));
    let second = StorageKey::new(Origin::with_port("https", "example.com", 443));
    let first_context = context_for(&first);
    let second_context = context_for(&second);
    let first = first_context.trusted_storage_key();
    let second = second_context.trusted_storage_key();
    let mut storage = OriginStorage::new();

    storage.set_item(&first, "theme", "dark");

    assert_eq!(storage.get_item(&first, "theme"), Some("dark"));
    assert_eq!(storage.get_item(&second, "theme"), Some("dark"));
}
