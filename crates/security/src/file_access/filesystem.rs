//! Filesystem abstraction used to open files approved by security policy.

use std::{fs, io::Read};

use super::FileAccessError;

/// Materialized data read from an authorized file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileResource {
    /// File contents in byte order.
    pub bytes: Vec<u8>,
    /// Optional media type determined by the filesystem owner.
    pub media_type: Option<String>,
}

/// An opened regular file accepted by the capability-backed filesystem.
///
/// The handle is intentionally private. Callers can materialize it with
/// [`Self::read_all`] now, while a future streaming API can retain ownership of
/// the same handle. Callers must not reopen the authorized pathname.
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

    /// Reads the complete file through this opened handle.
    ///
    /// Consumes the handle, closing it after the read completes or fails. The
    /// returned bytes are therefore from the file that the backend opened,
    /// rather than from a later pathname lookup.
    pub fn read_all(mut self) -> Result<FileResource, FileAccessError> {
        let mut bytes = Vec::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            match self.read_chunk(&mut buffer)? {
                0 => break,
                count => bytes.extend_from_slice(&buffer[..count]),
            }
        }
        Ok(FileResource {
            bytes,
            media_type: self.media_type,
        })
    }
}
