//! Reusable fixtures for Storage contract tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use common::ipc::{StorageRequest, StorageResponse};
use security::{Origin, StorageKey};
use storage::{PersistentOriginStorage, StorageLimits};

/// The default origin used by Storage tests.
pub fn primary_origin() -> Origin {
    Origin::with_port("https", "example.com", 443)
}

/// A separate origin used to verify isolation.
pub fn other_origin() -> Origin {
    Origin::with_port("https", "other.example", 443)
}

/// Builds a storage key for an origin.
pub fn storage_key_for(origin: Origin) -> StorageKey {
    StorageKey::new(origin)
}

/// Creates a uniquely named temporary storage directory.
pub fn test_directory(label: &str) -> TestDirectory {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after the Unix epoch")
        .as_nanos();
    TestDirectory(std::env::temp_dir().join(format!("byui-storage-{label}-{suffix}")))
}

/// Owns a test directory and removes it when the test finishes.
pub struct TestDirectory(PathBuf);

impl TestDirectory {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opens a persistent store using the default limits.
pub fn persistent_storage(directory: &TestDirectory) -> PersistentOriginStorage {
    PersistentOriginStorage::open(directory.path()).expect("test storage should open")
}

/// Opens a store with deliberately small limits for quota tests.
pub fn quota_storage(directory: &TestDirectory) -> PersistentOriginStorage {
    PersistentOriginStorage::open_with_limits(directory.path(), StorageLimits::new(4, 8))
        .expect("quota test storage should open")
}

/// A valid read request for fixture-based IPC tests.
pub fn get_request(key: &str) -> StorageRequest {
    StorageRequest::GetItem { key: key.into() }
}

/// A valid write request for fixture-based IPC tests.
pub fn set_request(key: &str, value: &str) -> StorageRequest {
    StorageRequest::SetItem {
        key: key.into(),
        value: value.into(),
    }
}

/// A successful response fixture for a stored value.
pub fn item_response(value: &str) -> StorageResponse {
    StorageResponse::Item(Some(value.into()))
}
