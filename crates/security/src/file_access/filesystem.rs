//! Filesystem abstraction used after a path has passed security policy.

use std::{fs, sync::Arc};

use super::{AuthorizedPath, FileAccessError};

/// Materialized data read from an authorized file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileResource {
    /// File contents in byte order.
    pub bytes: Vec<u8>,
    /// Optional media type determined by the filesystem owner.
    pub media_type: Option<String>,
}

/// Reads files after authorization has produced an [`AuthorizedPath`].
pub trait FileSystem: Send + Sync {
    /// Reads the complete file resource.
    fn read(&self, path: &AuthorizedPath) -> Result<FileResource, FileAccessError>;
}

/// Standard-library filesystem implementation for the initial browser shell.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalFileSystem;

impl FileSystem for LocalFileSystem {
    fn read(&self, path: &AuthorizedPath) -> Result<FileResource, FileAccessError> {
        Ok(FileResource {
            bytes: fs::read(path.as_path()).map_err(FileAccessError::from)?,
            media_type: None,
        })
    }
}

/// Convenience handle for sharing a filesystem implementation.
pub type SharedFileSystem = Arc<dyn FileSystem>;
