//! Public-contract tests for `storage`.
//!
//! Run ignored tests with `cargo test -p storage -- --ignored` to see the backlog.

use storage::LocalStorage;

#[test]
fn public_api_round_trip() {
    let mut store = LocalStorage::new();
    store.set_item("theme", "dark");
    assert_eq!(store.get_item("theme"), Some("dark"));
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
