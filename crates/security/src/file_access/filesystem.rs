//! Filesystem abstraction used to open files approved by security policy.

use std::{fs, io::Read};

use super::FileAccessError;

/// An opened regular file accepted by the capability-backed filesystem.
///
/// The handle is intentionally private and supports chunked reads through the
/// same file selected by the capability. Callers must not reopen the
/// authorized pathname.
#[derive(Debug)]
pub struct OpenedFile {
    pub(crate) file: fs::File,
    pub(crate) media_type: Option<String>,
    pub(crate) length: u64,
}

impl OpenedFile {
    /// Reads the next chunk through this opened handle.
    ///
    /// The returned count is zero at end of file. The operation is blocking;
    /// callers running on an asynchronous executor must perform it on a
    /// blocking executor. The read always uses the retained handle and never
    /// reopens the authorized pathname.
    pub fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, FileAccessError> {
        self.file.read(buffer).map_err(FileAccessError::from)
    }

    /// Returns the file length observed when the handle was opened.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Returns the filesystem-provided media type, when one is available.
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }
}
