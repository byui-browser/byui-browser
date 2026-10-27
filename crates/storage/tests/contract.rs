//! Public-contract tests for `storage`.
//!
//! Run ignored tests with `cargo test -p storage -- --ignored` to see the backlog.

use security::{Origin, RendererSecurityContext, StoragePermission};
use storage::{LocalStorage, OriginStorage};

#[test]
fn public_api_round_trip() {
    let mut store = LocalStorage::new();
    store.set_item("theme", "dark");
    assert_eq!(store.get_item("theme"), Some("dark"));
}

#[test]
fn public_api_is_origin_scoped() {
    let first = RendererSecurityContext::new(
        Origin::with_port("https", "example.com", 443),
        StoragePermission::Allowed,
    );
    let second = RendererSecurityContext::new(
        Origin::with_port("https", "other.example", 443),
        StoragePermission::Allowed,
    );
    let mut storage = OriginStorage::new();

    storage
        .set_item(&first, "theme", "dark")
        .expect("first context can write");

    assert_eq!(
        storage
            .get_item(&first, "theme")
            .expect("first context can read"),
        Some("dark")
    );
    assert_eq!(
        storage
            .get_item(&second, "theme")
            .expect("second context can read its own storage"),
        None
    );
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
