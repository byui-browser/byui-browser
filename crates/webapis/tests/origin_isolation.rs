use js::{Realm, Value};
use security::{Origin, RendererSecurityContext, StoragePermission};
use storage::{OriginStorage, SharedStorage, shared_storage};
use webapis::local_storage::register_local_storage;

fn context(
    scheme: &str,
    host: &str,
    port: u16,
    permission: StoragePermission,
) -> RendererSecurityContext {
    RendererSecurityContext::new(Origin::with_port(scheme, host, port), permission)
}

fn realm(storage: SharedStorage, context: RendererSecurityContext) -> Realm {
    let mut realm = Realm::new();
    register_local_storage(&mut realm, storage, context)
        .expect("localStorage functions should register");
    realm
}

fn set_item(realm: &mut Realm, key: &str, value: &str) {
    realm
        .call_global(
            "localStorage.setItem",
            &[Value::String(key.into()), Value::String(value.into())],
        )
        .expect("localStorage.setItem should succeed");
}

fn get_item(realm: &mut Realm, key: &str) -> Result<Value, js::JsError> {
    realm.call_global("localStorage.getItem", &[Value::String(key.into())])
}

#[test]
fn same_origin_contexts_share_storage() {
    let storage = shared_storage(OriginStorage::new());
    let mut first = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let mut second = realm(
        storage,
        context("https", "example.com", 443, StoragePermission::Allowed),
    );

    set_item(&mut first, "theme", "dark");

    assert_eq!(
        get_item(&mut second, "theme"),
        Ok(Value::String("dark".into()))
    );
}

#[test]
fn cross_origin_reads_are_isolated() {
    let storage = shared_storage(OriginStorage::new());
    let mut example = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let mut other = realm(
        storage,
        context("https", "other.example", 443, StoragePermission::Allowed),
    );

    set_item(&mut example, "secret", "example-value");

    assert_eq!(get_item(&mut other, "secret"), Ok(Value::Null));
}

#[test]
fn cross_origin_writes_do_not_overwrite() {
    let storage = shared_storage(OriginStorage::new());
    let mut example = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let mut other = realm(
        storage,
        context("https", "other.example", 443, StoragePermission::Allowed),
    );

    set_item(&mut example, "shared-name", "example-value");
    set_item(&mut other, "shared-name", "other-value");

    assert_eq!(
        get_item(&mut example, "shared-name"),
        Ok(Value::String("example-value".into()))
    );
    assert_eq!(
        get_item(&mut other, "shared-name"),
        Ok(Value::String("other-value".into()))
    );
}

#[test]
fn different_schemes_use_different_storage() {
    let storage = shared_storage(OriginStorage::new());
    let mut https = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let mut http = realm(
        storage,
        context("http", "example.com", 80, StoragePermission::Allowed),
    );

    set_item(&mut https, "transport", "secure");
    set_item(&mut http, "transport", "insecure");

    assert_eq!(
        get_item(&mut https, "transport"),
        Ok(Value::String("secure".into()))
    );
    assert_eq!(
        get_item(&mut http, "transport"),
        Ok(Value::String("insecure".into()))
    );
}

#[test]
fn different_ports_use_different_storage() {
    let storage = shared_storage(OriginStorage::new());
    let mut default_port = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let mut alternate_port = realm(
        storage,
        context("https", "example.com", 8443, StoragePermission::Allowed),
    );

    set_item(&mut default_port, "port", "default");
    set_item(&mut alternate_port, "port", "alternate");

    assert_eq!(
        get_item(&mut default_port, "port"),
        Ok(Value::String("default".into()))
    );
    assert_eq!(
        get_item(&mut alternate_port, "port"),
        Ok(Value::String("alternate".into()))
    );
}

#[test]
fn denied_context_cannot_read_storage() {
    let storage = shared_storage(OriginStorage::new());
    let mut allowed = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let mut denied = realm(
        storage,
        context("https", "example.com", 443, StoragePermission::Denied),
    );

    set_item(&mut allowed, "key", "original");

    assert_eq!(
        get_item(&mut denied, "key").unwrap_err().to_string(),
        "Uncaught SecurityError: localStorage read access denied"
    );
    assert_eq!(
        get_item(&mut allowed, "key"),
        Ok(Value::String("original".into()))
    );
}

#[test]
fn denied_context_cannot_write_storage() {
    let storage = shared_storage(OriginStorage::new());
    let mut allowed = realm(
        storage.clone(),
        context("https", "example.com", 443, StoragePermission::Allowed),
    );
    let denied = realm(
        storage,
        context("https", "example.com", 443, StoragePermission::Denied),
    );

    set_item(&mut allowed, "key", "original");

    assert_eq!(
        denied
            .call_global(
                "localStorage.setItem",
                &[
                    Value::String("key".into()),
                    Value::String("replacement".into())
                ],
            )
            .unwrap_err()
            .to_string(),
        "Uncaught SecurityError: localStorage write access denied"
    );
    assert_eq!(
        get_item(&mut allowed, "key"),
        Ok(Value::String("original".into()))
    );
}
