//! Errors produced while authorizing or reading a local file.

use std::{fmt, io};

/// Stable categories for browser-owned file access failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileAccessError {
    /// The requested path does not exist.
    Missing,
    /// The security policy denied access to the requested path.
    Denied,
    /// The requested path could not be safely normalized.
    InvalidPath,
    /// The opened path is not a regular file that can be served.
    UnsupportedFile,
    /// The filesystem returned an I/O failure unrelated to authorization.
    Io,
}

impl fmt::Display for FileAccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Missing => "file is missing",
            Self::Denied => "file access was denied",
            Self::InvalidPath => "file path is invalid",
            Self::UnsupportedFile => "file type is unsupported",
            Self::Io => "file I/O failed",
        };
        f.write_str(message)
    }
}

impl std::error::Error for FileAccessError {}

impl From<io::Error> for FileAccessError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::Missing,
            io::ErrorKind::PermissionDenied => Self::Denied,
            _ => Self::Io,
        }
    }
}
