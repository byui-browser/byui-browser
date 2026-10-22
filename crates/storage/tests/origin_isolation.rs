use security::{Origin, RendererSecurityContext, StoragePermission};
use storage::{OriginStorage, StorageAccessError};

fn context(host: &str, permission: StoragePermission) -> RendererSecurityContext {
    RendererSecurityContext::new(Origin::new("https", host, None), permission)
}

#[test]
fn same_origin_contexts_share_storage() {
    let first = context("example.test", StoragePermission::Allowed);
    let second = context("example.test", StoragePermission::Allowed);
    let mut storage = OriginStorage::new();

    storage
        .set_item(&first, "theme", "dark")
        .expect("first context can write");

    assert_eq!(
        storage
            .get_item(&second, "theme")
            .expect("same-origin context can read"),
        Some("dark")
    );
}

#[test]
fn cross_origin_reads_do_not_expose_values() {
    let origin_a = context("a.example.test", StoragePermission::Allowed);
    let origin_b = context("b.example.test", StoragePermission::Allowed);
    let mut storage = OriginStorage::new();

    storage
        .set_item(&origin_a, "secret", "origin-a-value")
        .expect("origin A can write");

    assert_eq!(
        storage
            .get_item(&origin_b, "secret")
            .expect("origin B is allowed to read its own storage"),
        None
    );
}

#[test]
fn cross_origin_writes_do_not_overwrite_values() {
    let origin_a = context("a.example.test", StoragePermission::Allowed);
    let origin_b = context("b.example.test", StoragePermission::Allowed);
    let mut storage = OriginStorage::new();

    storage
        .set_item(&origin_a, "shared-name", "origin-a-value")
        .expect("origin A can write");
    storage
        .set_item(&origin_b, "shared-name", "origin-b-value")
        .expect("origin B can write its own storage");

    assert_eq!(
        storage
            .get_item(&origin_a, "shared-name")
            .expect("origin A can read its own storage"),
        Some("origin-a-value")
    );
}

#[test]
fn denied_context_cannot_read_or_write_storage() {
    let allowed = context("example.test", StoragePermission::Allowed);
    let denied = context("example.test", StoragePermission::Denied);
    let mut storage = OriginStorage::new();
    storage
        .set_item(&allowed, "key", "original")
        .expect("allowed context can write");

    assert_eq!(
        storage.get_item(&denied, "key"),
        Err(StorageAccessError::ReadDenied)
    );
    assert_eq!(
        storage.set_item(&denied, "key", "replacement"),
        Err(StorageAccessError::WriteDenied)
    );
    assert_eq!(
        storage
            .get_item(&allowed, "key")
            .expect("allowed context can still read"),
        Some("original")
    );
}

#[test]
fn different_storage_keys_select_isolated_stores() {
    let origin_a = context("a.example.test", StoragePermission::Allowed);
    let origin_b = context("b.example.test", StoragePermission::Allowed);
    let mut storage = OriginStorage::new();

    assert_ne!(origin_a.storage_key(), origin_b.storage_key());
    storage
        .set_item(&origin_a, "key", "origin-a-value")
        .expect("origin A can write");

    assert_eq!(
        storage
            .get_item(&origin_b, "key")
            .expect("origin B can read its own storage"),
        None
    );
}
