//! Public-contract tests for the typed storage IPC messages.

use common::ipc::{StorageError, StorageRequest, StorageResponse};

#[test]
fn storage_messages_are_cloneable_and_comparable() {
    let request = StorageRequest::SetItem {
        key: "session".into(),
        value: "abc".into(),
    };
    let copied = request.clone();

    assert_eq!(request, copied);
}

#[test]
fn storage_errors_are_part_of_the_response_contract() {
    let response = StorageResponse::Error(StorageError::QuotaExceeded);

    assert_eq!(
        response,
        StorageResponse::Error(StorageError::QuotaExceeded)
    );
}
