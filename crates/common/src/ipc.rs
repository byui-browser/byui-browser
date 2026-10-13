//! Typed messages exchanged between the renderer and storage systems.
//!
//! These types describe the storage contract. They do not implement a
//! transport or serialization format yet; those can be added once the teams
//! agree on the process channel.

/// A request sent by a renderer to the storage system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageRequest {
    /// Read the value associated with `key`.
    GetItem {
        /// Storage key to look up.
        key: String,
    },
    /// Store `value` under `key`.
    SetItem {
        /// Storage key to update.
        key: String,
        /// New value for the key.
        value: String,
    },
    /// Remove the value associated with `key`.
    RemoveItem {
        /// Storage key to remove.
        key: String,
    },
    /// Remove every item in the selected storage area.
    Clear,
    /// Ask how many items are in the selected storage area.
    Length,
}

/// A response returned by the storage system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageResponse {
    /// Result of a [`StorageRequest::GetItem`] request.
    Item(Option<String>),
    /// Result of a [`StorageRequest::Length`] request.
    Length(usize),
    /// Indicates that a mutating request completed successfully.
    Success,
    /// Indicates that the storage system could not complete the request.
    Error(StorageError),
}

/// A typed failure that can cross the storage IPC boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// The request was not valid for the storage API.
    InvalidRequest,
    /// The storage area cannot accept more data.
    QuotaExceeded,
    /// The renderer is not allowed to access the selected storage area.
    AccessDenied,
    /// The storage backend is temporarily unavailable.
    BackendUnavailable,
}

#[cfg(test)]
mod tests {
    use super::{StorageError, StorageRequest, StorageResponse};

    #[test]
    fn requests_can_describe_each_storage_operation() {
        let requests = [
            StorageRequest::GetItem {
                key: "theme".into(),
            },
            StorageRequest::SetItem {
                key: "theme".into(),
                value: "dark".into(),
            },
            StorageRequest::RemoveItem {
                key: "theme".into(),
            },
            StorageRequest::Clear,
            StorageRequest::Length,
        ];

        assert_eq!(requests.len(), 5);
    }

    #[test]
    fn responses_can_represent_values_success_and_errors() {
        assert_eq!(
            StorageResponse::Item(Some("dark".into())),
            StorageResponse::Item(Some("dark".into()))
        );
        assert_eq!(StorageResponse::Item(None), StorageResponse::Item(None));
        assert_eq!(StorageResponse::Length(2), StorageResponse::Length(2));
        assert_eq!(StorageResponse::Success, StorageResponse::Success);
        assert_eq!(
            StorageResponse::Error(StorageError::AccessDenied),
            StorageResponse::Error(StorageError::AccessDenied)
        );
    }
}
